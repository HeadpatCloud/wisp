use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::commands::store_cmds::change;
use crate::error::{AppError, AppResult};
use crate::net::address_key;
use crate::ssh::ssh_config::parse_ssh_config;
use crate::store::hosts::{
    Connection, ConnectionKind, Host, HostsFile, Login, OwnLogin, SshConn, SshKey, THIS_COMPUTER,
};
use crate::store::model::{AuthMethod, IconRef};
use crate::store::{next_order, Store};

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportCandidate {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub key_path: Option<String>,
    pub jump_host_alias: Option<String>,
    pub status: CandidateStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CandidateStatus {
    New,
    AddsTo(HostRef),
    AlreadyThere(HostRef),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HostRef {
    pub host_name: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SshImportSummary {
    pub hosts: u32,
    pub connections: u32,
}

fn login_user<'a>(file: &'a HostsFile, login: &'a Login) -> Option<&'a str> {
    match login {
        Login::Own(o) => Some(&o.username),
        Login::Saved(s) => file.logins.iter().find(|l| l.id == s.login_id).map(|l| l.username.as_str()),
    }
}

fn matching_host<'a>(file: &'a HostsFile, c: &ImportCandidate) -> Option<&'a Host> {
    let key = address_key(&c.host);
    file.hosts.iter().find(|h| h.id != THIS_COMPUTER && address_key(&h.address) == key)
}

// The SSH connection on the matching host that already is this entry: same port and user.
fn same_ssh<'a>(file: &'a HostsFile, host: &'a Host, c: &ImportCandidate) -> Option<&'a Connection> {
    host.connections.iter().find(|conn| {
        matches!(&conn.kind, ConnectionKind::Ssh(s)
            if s.port == c.port && login_user(file, &s.login) == Some(c.username.as_str()))
    })
}

pub(crate) fn status_of(file: &HostsFile, c: &ImportCandidate) -> CandidateStatus {
    match matching_host(file, c) {
        None => CandidateStatus::New,
        Some(h) => {
            let r = HostRef { host_name: h.name.clone() };
            if same_ssh(file, h, c).is_some() {
                CandidateStatus::AlreadyThere(r)
            } else {
                CandidateStatus::AddsTo(r)
            }
        }
    }
}

fn key_id_for(file: &mut HostsFile, path: &str) -> String {
    if let Some(k) = file.keys.iter().find(|k| k.path == path) {
        return k.id.clone();
    }
    let id = Uuid::new_v4().to_string();
    file.keys.push(SshKey { id: id.clone(), path: path.into(), secret_id: None, extra: Default::default() });
    id
}

// Entries are matched one after another against the file as it grows, so two aliases of one
// server end up on one host. ProxyJump is linked last, once every alias has its connection.
pub(crate) fn apply_candidates(file: &HostsFile, picks: &[ImportCandidate]) -> (HostsFile, SshImportSummary) {
    let mut file = file.clone();
    let mut summary = SshImportSummary { hosts: 0, connections: 0 };
    let mut alias_conn: HashMap<&str, String> = HashMap::new();
    let mut jumps: Vec<(String, &str)> = Vec::new();
    for c in picks {
        if let Some(h) = matching_host(&file, c) {
            if let Some(there) = same_ssh(&file, h, c) {
                alias_conn.insert(&c.name, there.id.clone());
                continue;
            }
        }
        let key_ids: Vec<String> = c.key_path.iter().map(|p| key_id_for(&mut file, p)).collect();
        let auth_method = if key_ids.is_empty() { AuthMethod::Password } else { AuthMethod::Key };
        let id = Uuid::new_v4().to_string();
        let mut conn = Connection {
            id: id.clone(),
            label: None,
            address: None,
            kind: ConnectionKind::Ssh(SshConn {
                port: c.port,
                login: Login::Own(OwnLogin { username: c.username.clone(), auth_method, key_ids, secret_id: None, extra: Default::default() }),
                jump_host_id: None,
                tunnels: vec![],
                appearance: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        };
        let target = matching_host(&file, c).map(|h| h.id.clone());
        match target {
            Some(host_id) => {
                let host = file.hosts.iter_mut().find(|h| h.id == host_id).expect("matched above");
                conn.label = (c.name != host.name).then(|| c.name.clone());
                host.connections.push(conn);
            }
            None => {
                let order = next_order(&file, None);
                file.hosts.push(Host {
                    id: Uuid::new_v4().to_string(),
                    name: c.name.clone(),
                    address: c.host.clone(),
                    folder_id: None,
                    tag_ids: vec![],
                    icon: IconRef::default(),
                    color: None,
                    order,
                    connections: vec![conn],
                    extra: Default::default(),
                });
                summary.hosts += 1;
            }
        }
        summary.connections += 1;
        alias_conn.insert(&c.name, id.clone());
        if let Some(alias) = &c.jump_host_alias {
            jumps.push((id, alias));
        }
    }
    for (id, alias) in jumps {
        let jump = alias_conn.get(alias).cloned().or_else(|| {
            let host = file.hosts.iter().find(|h| h.name == alias)?;
            host.connections.iter().find(|c| matches!(c.kind, ConnectionKind::Ssh(_))).map(|c| c.id.clone())
        });
        let conn = file.hosts.iter_mut().flat_map(|h| h.connections.iter_mut()).find(|c| c.id == id);
        if let Some(Connection { kind: ConnectionKind::Ssh(s), .. }) = conn {
            s.jump_host_id = jump;
        }
    }
    (file, summary)
}

#[tauri::command]
#[specta::specta]
pub async fn import_ssh_config(
    app: AppHandle,
    store: State<'_, Mutex<Store>>,
    path: Option<String>,
) -> AppResult<Vec<ImportCandidate>> {
    let home = app.path().home_dir().map_err(|e| AppError::Io(e.to_string()))?;
    let config_path = match path {
        Some(p) => PathBuf::from(p),
        None => home.join(".ssh").join("config"),
    };
    let text = std::fs::read_to_string(&config_path)
        .map_err(|e| AppError::Io(format!("{}: {e}", config_path.display())))?;

    let file = store.lock().map_err(|_| AppError::Internal("store lock poisoned".into()))?.hosts();
    let candidates = parse_ssh_config(&text)
        .into_iter()
        .map(|h| {
            let key_path = h.identity_file.map(|f| {
                if let Some(rest) = f.strip_prefix("~/").or_else(|| f.strip_prefix("~\\")) {
                    home.join(rest).to_string_lossy().into_owned()
                } else {
                    f
                }
            });
            let mut c = ImportCandidate {
                host: h.host_name.unwrap_or_else(|| h.name.clone()),
                port: h.port.unwrap_or(22),
                username: h.user.unwrap_or_default(),
                key_path,
                jump_host_alias: h.proxy_jump,
                name: h.name,
                status: CandidateStatus::New,
            };
            c.status = status_of(&file, &c);
            c
        })
        .collect();
    Ok(candidates)
}

// The status the page showed is worked out again here, against the store as it is now.
#[tauri::command]
#[specta::specta]
pub fn ssh_config_apply(
    app: AppHandle,
    store: State<'_, Mutex<Store>>,
    candidates: Vec<ImportCandidate>,
) -> AppResult<SshImportSummary> {
    change(&app, &store, |s| {
        let (file, summary) = apply_candidates(&s.hosts(), &candidates);
        s.replace_hosts(file)?;
        Ok(summary)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::hosts::fixtures::{file, host, own, ssh, vnc};
    use crate::store::hosts::{ConnectionKind, HostsFile, SshKey};

    fn cand(name: &str, host: &str, user: &str, key: Option<&str>, jump: Option<&str>) -> ImportCandidate {
        ImportCandidate {
            name: name.into(),
            host: host.into(),
            port: 22,
            username: user.into(),
            key_path: key.map(Into::into),
            jump_host_alias: jump.map(Into::into),
            status: CandidateStatus::New,
        }
    }

    fn named<'a>(f: &'a HostsFile, name: &str) -> &'a crate::store::hosts::Host {
        f.hosts.iter().find(|h| h.name == name).unwrap_or_else(|| panic!("no host {name}"))
    }

    fn jump_of(f: &HostsFile, host_name: &str) -> Option<String> {
        match &named(f, host_name).connections.last().unwrap().kind {
            ConnectionKind::Ssh(s) => s.jump_host_id.clone(),
            _ => None,
        }
    }

    #[test]
    fn config_entries_add_to_matching_hosts_or_make_new_ones() {
        let local = file(vec![host("n", "nas", "nas.local", vec![vnc("v", None)])]);
        let picks = [cand("nas-ssh", "NAS.local", "admin", None, None), cand("web", "10.0.0.2", "me", None, None)];
        assert_eq!(status_of(&local, &picks[0]), CandidateStatus::AddsTo(HostRef { host_name: "nas".into() }));
        assert_eq!(status_of(&local, &picks[1]), CandidateStatus::New);
        let (after, summary) = apply_candidates(&local, &picks);
        let nas = named(&after, "nas");
        assert_eq!(nas.connections.len(), 2);
        assert_eq!(nas.connections[1].label.as_deref(), Some("nas-ssh"));
        assert_eq!(named(&after, "web").address, "10.0.0.2");
        assert_eq!((summary.hosts, summary.connections), (1, 2));
    }

    #[test]
    fn proxy_jump_links_an_imported_or_existing_alias() {
        let local = file(vec![host("b", "bastion", "bastion.corp", vec![ssh("b1", 22, own("ops", None))])]);
        let picks = [
            cand("app", "10.0.0.5", "me", None, Some("bastion")),
            cand("db", "10.0.0.6", "me", None, Some("app")),
            cand("x", "10.0.0.7", "me", None, Some("nowhere")),
        ];
        let (after, _) = apply_candidates(&local, &picks);
        assert_eq!(jump_of(&after, "app").as_deref(), Some("b1"));
        let app_conn = named(&after, "app").connections[0].id.clone();
        assert_eq!(jump_of(&after, "db"), Some(app_conn));
        assert_eq!(jump_of(&after, "x"), None);
    }

    #[test]
    fn an_entry_already_there_is_skipped() {
        let local = file(vec![host("n", "nas", "nas.local", vec![ssh("s", 22, own("admin", None))])]);
        let pick = cand("nas", "nas.local", "admin", None, None);
        assert_eq!(status_of(&local, &pick), CandidateStatus::AlreadyThere(HostRef { host_name: "nas".into() }));
        let (after, summary) = apply_candidates(&local, &[pick]);
        assert_eq!(named(&after, "nas").connections.len(), 1);
        assert_eq!((summary.hosts, summary.connections), (0, 0));
    }

    #[test]
    fn a_key_path_is_listed_once() {
        let mut local = file(vec![]);
        local.keys.push(SshKey { id: "k1".into(), path: "/home/me/.ssh/id".into(), secret_id: None, extra: Default::default() });
        let picks = [
            cand("a", "10.0.0.1", "me", Some("/home/me/.ssh/id"), None),
            cand("b", "10.0.0.2", "me", Some("/home/me/.ssh/id"), None),
            cand("c", "10.0.0.3", "me", Some("/home/me/.ssh/other"), None),
        ];
        let (after, _) = apply_candidates(&local, &picks);
        assert_eq!(after.keys.len(), 2);
        let keys_of = |name: &str| match &named(&after, name).connections[0].kind {
            ConnectionKind::Ssh(s) => match &s.login {
                crate::store::hosts::Login::Own(o) => (o.auth_method, o.key_ids.clone()),
                _ => panic!("saved login"),
            },
            _ => panic!("not ssh"),
        };
        assert_eq!(keys_of("a"), (crate::store::model::AuthMethod::Key, vec!["k1".to_string()]));
        assert_eq!(keys_of("b").1, vec!["k1".to_string()]);
        assert_ne!(keys_of("c").1, vec!["k1".to_string()]);
    }
}
