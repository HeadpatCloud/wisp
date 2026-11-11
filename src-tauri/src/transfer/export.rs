use std::collections::HashSet;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::bundle::{KeyFile, PayloadV3};
use super::{Env, ExportOptions, ExportSelection};
use crate::store::hosts::{
    ConnectionKind, HostsFile, Login, OwnLogin, S3Conn, VncConn, THIS_COMPUTER,
};

fn file_name(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or("key");
    // A key imported earlier is stored as `<uuid>-name`; exported like that, the prefix would
    // grow with every hop.
    name.get(..36)
        .filter(|id| Uuid::parse_str(id).is_ok())
        .and_then(|_| name[36..].strip_prefix('-'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(name)
        .to_string()
}

pub fn build(
    file: &HostsFile,
    env: &dyn Env,
    sel: &ExportSelection,
    opts: &ExportOptions,
) -> (PayloadV3, Vec<String>) {
    let mut warnings = Vec::new();

    let mut picked_folders: HashSet<&str> = sel.folder_ids.iter().map(String::as_str).collect();
    loop {
        let children: Vec<&str> = file
            .folders
            .iter()
            .filter(|f| f.parent_id.as_deref().is_some_and(|p| picked_folders.contains(p)))
            .map(|f| f.id.as_str())
            .filter(|id| !picked_folders.contains(id))
            .collect();
        if children.is_empty() {
            break;
        }
        picked_folders.extend(children);
    }

    let mut host_ids: HashSet<&str> = sel.host_ids.iter().map(String::as_str).collect();
    for h in &file.hosts {
        if h.folder_id.as_deref().is_some_and(|f| picked_folders.contains(f)) {
            host_ids.insert(&h.id);
        }
    }
    host_ids.remove(THIS_COMPUTER);
    // A connection is useless on the other machine without its jump chain.
    loop {
        let jumps: Vec<&str> = file
            .hosts
            .iter()
            .filter(|h| host_ids.contains(h.id.as_str()))
            .flat_map(|h| &h.connections)
            .filter_map(|c| match &c.kind {
                ConnectionKind::Ssh(s) => s.jump_host_id.as_deref(),
                _ => None,
            })
            .filter_map(|j| file.hosts.iter().find(|h| h.connections.iter().any(|c| c.id == j)))
            .filter(|h| h.id != THIS_COMPUTER && !host_ids.contains(h.id.as_str()))
            .map(|h| h.id.as_str())
            .collect();
        if jumps.is_empty() {
            break;
        }
        for id in jumps {
            if host_ids.insert(id) {
                let name = &file.hosts.iter().find(|h| h.id == id).expect("found above").name;
                warnings.push(format!("Also exported jump host \"{name}\""));
            }
        }
    }
    let hosts: Vec<_> = file.hosts.iter().filter(|h| host_ids.contains(h.id.as_str())).cloned().collect();

    let mut folder_ids = picked_folders;
    folder_ids.extend(hosts.iter().filter_map(|h| h.folder_id.as_deref()));
    let mut frontier: Vec<&str> = folder_ids.iter().copied().collect();
    while let Some(id) = frontier.pop() {
        let parent = file.folders.iter().find(|f| f.id == id).and_then(|f| f.parent_id.as_deref());
        if let Some(parent) = parent {
            if folder_ids.insert(parent) {
                frontier.push(parent);
            }
        }
    }

    let logins_used: HashSet<&str> = hosts
        .iter()
        .flat_map(|h| &h.connections)
        .flat_map(|c| c.kind.logins())
        .filter_map(|l| match l {
            Login::Saved(s) => Some(s.login_id.as_str()),
            Login::Own(_) => None,
        })
        .collect();
    let logins: Vec<_> =
        file.logins.iter().filter(|l| logins_used.contains(l.id.as_str())).cloned().collect();
    let keys_used: HashSet<String> = hosts
        .iter()
        .flat_map(|h| &h.connections)
        .flat_map(|c| c.kind.logins())
        .filter_map(|l| match l {
            Login::Own(o) => Some(&o.key_ids),
            Login::Saved(_) => None,
        })
        .chain(logins.iter().map(|l| &l.key_ids))
        .flatten()
        .cloned()
        .collect();
    let tags_used: HashSet<&str> = hosts.iter().flat_map(|h| &h.tag_ids).map(String::as_str).collect();

    let mut payload = PayloadV3 {
        folders: file.folders.iter().filter(|f| folder_ids.contains(f.id.as_str())).cloned().collect(),
        logins,
        keys: file.keys.iter().filter(|k| keys_used.contains(&k.id)).cloned().collect(),
        tags: file.tags.iter().filter(|t| tags_used.contains(t.id.as_str())).cloned().collect(),
        hosts,
        ..Default::default()
    };

    if opts.include_secrets {
        let mut owners: Vec<(String, &str)> = Vec::new();
        for h in &payload.hosts {
            for c in &h.connections {
                for login in c.kind.logins() {
                    if let Login::Own(OwnLogin { secret_id: Some(id), .. }) = login {
                        owners.push((h.name.clone(), id));
                    }
                }
                match &c.kind {
                    ConnectionKind::S3(S3Conn { secret_id: Some(id), .. })
                    | ConnectionKind::Vnc(VncConn { secret_id: Some(id), .. }) => {
                        owners.push((h.name.clone(), id))
                    }
                    _ => {}
                }
            }
        }
        for l in &payload.logins {
            owners.extend(l.secret_id.as_deref().map(|id| (l.name.clone(), id)));
        }
        for k in &payload.keys {
            owners.extend(k.secret_id.as_deref().map(|id| (file_name(&k.path), id)));
        }
        let mut secrets = Vec::new();
        for (name, id) in owners {
            match env.secret(id).and_then(|v| String::from_utf8(v.to_vec()).ok()) {
                Some(value) => secrets.push((id.to_string(), Zeroizing::new(value))),
                None => warnings.push(format!("Couldn't read a saved password for \"{name}\"")),
            }
        }
        payload.secrets.extend(secrets);
    }
    if opts.include_keys {
        let mut key_files = Vec::new();
        for k in &payload.keys {
            match env.read_file(&k.path) {
                Some(bytes) => key_files.push((
                    k.path.clone(),
                    KeyFile {
                        file_name: file_name(&k.path),
                        data: Zeroizing::new(STANDARD.encode(bytes)),
                    },
                )),
                None => warnings.push(format!("Couldn't read key file {}", k.path)),
            }
        }
        payload.key_files.extend(key_files);
    }
    (payload, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::hosts::fixtures::{conn, file, folder, host, own, ssh, vnc};
    use crate::store::hosts::{
        ConnectionKind, Login, OwnLogin, RdpConn, RdpGatewayConn, SavedLogin, SavedLoginRef,
        SshConn, SshKey, THIS_COMPUTER,
    };
    use crate::store::model::AuthMethod;
    use crate::transfer::MapEnv;

    fn keyed(id: &str, key: &str) -> crate::store::hosts::Connection {
        ssh(
            id,
            22,
            Login::Own(OwnLogin {
                username: "u".into(),
                auth_method: AuthMethod::Key,
                key_ids: vec![key.into()],
                secret_id: None,
                extra: Default::default(),
            }),
        )
    }

    fn jumping(id: &str, key: &str, jump: &str) -> crate::store::hosts::Connection {
        let mut c = keyed(id, key);
        if let ConnectionKind::Ssh(SshConn { jump_host_id, .. }) = &mut c.kind {
            *jump_host_id = Some(jump.into());
        }
        c
    }

    fn key(id: &str) -> SshKey {
        SshKey {
            id: id.into(),
            path: format!("/keys/{id}"),
            secret_id: Some(format!("pp-{id}")),
            extra: Default::default(),
        }
    }

    // root > child holds web (behind bastion); other is empty.
    fn data() -> HostsFile {
        let mut web = host("web", "web", "10.0.0.2", vec![jumping("web-ssh", "k-web", "b-ssh")]);
        web.folder_id = Some("child".into());
        let mut f = file(vec![
            host(THIS_COMPUTER, "This computer", "", vec![]),
            web,
            host("bastion", "bastion", "bastion.corp", vec![keyed("b-ssh", "k-b")]),
        ]);
        f.folders = vec![folder("root", None), folder("child", Some("root")), folder("other", None)];
        f.keys = vec![key("k-web"), key("k-b")];
        f
    }

    fn hosts_only(ids: &[&str]) -> ExportSelection {
        ExportSelection { host_ids: ids.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    fn host_ids(p: &PayloadV3) -> Vec<&str> {
        p.hosts.iter().map(|h| h.id.as_str()).collect()
    }

    #[test]
    fn jump_host_hosts_come_along_with_ancestor_folders() {
        let (payload, warnings) =
            build(&data(), &MapEnv::default(), &hosts_only(&["web"]), &ExportOptions::default());
        let folders: Vec<_> = payload.folders.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(folders, ["root", "child"]);
        assert_eq!(host_ids(&payload), ["web", "bastion"]);
        assert!(warnings.iter().any(|w| w.contains("bastion")));
        let keys: Vec<_> = payload.keys.iter().map(|k| k.id.as_str()).collect();
        assert_eq!(keys, ["k-web", "k-b"]);
    }

    #[test]
    fn a_shared_jump_host_is_warned_about_once() {
        let mut d = data();
        d.hosts.push(host("db", "db", "10.0.0.3", vec![jumping("db-ssh", "k-b", "b-ssh")]));
        let (payload, warnings) =
            build(&d, &MapEnv::default(), &hosts_only(&["web", "db"]), &ExportOptions::default());
        assert_eq!(payload.hosts.len(), 3);
        assert_eq!(warnings.iter().filter(|w| w.contains("bastion")).count(), 1);
    }

    #[test]
    fn selecting_a_folder_exports_its_subfolders() {
        let sel = ExportSelection { folder_ids: vec!["root".into()], ..Default::default() };
        let (payload, _) = build(&data(), &MapEnv::default(), &sel, &ExportOptions::default());
        assert_eq!(host_ids(&payload), ["web", "bastion"]);
        let folders: Vec<_> = payload.folders.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(folders, ["root", "child"]);
    }

    #[test]
    fn an_explicitly_selected_empty_folder_is_exported() {
        let sel = ExportSelection { folder_ids: vec!["other".into()], ..Default::default() };
        let (payload, _) = build(&data(), &MapEnv::default(), &sel, &ExportOptions::default());
        assert_eq!(payload.folders.len(), 1);
        assert!(payload.hosts.is_empty());
    }

    #[test]
    fn this_computer_is_never_exported() {
        let (payload, _) = build(
            &data(),
            &MapEnv::default(),
            &hosts_only(&[THIS_COMPUTER]),
            &ExportOptions::default(),
        );
        assert!(payload.hosts.is_empty());
    }

    #[test]
    fn saved_logins_and_keys_in_use_come_along() {
        let mut d = data();
        d.logins.push(SavedLogin {
            id: "l1".into(),
            name: "homelab".into(),
            username: "admin".into(),
            auth_method: AuthMethod::Key,
            key_ids: vec!["k-b".into()],
            secret_id: None,
            extra: Default::default(),
        });
        d.logins.push(SavedLogin {
            id: "unused".into(),
            name: "unused".into(),
            username: "x".into(),
            auth_method: AuthMethod::Password,
            key_ids: vec![],
            secret_id: None,
            extra: Default::default(),
        });
        let saved = Login::Saved(SavedLoginRef { login_id: "l1".into(), extra: Default::default() });
        d.hosts.push(host("nas", "nas", "nas.local", vec![ssh("nas-ssh", 22, saved)]));
        let (payload, _) =
            build(&d, &MapEnv::default(), &hosts_only(&["nas"]), &ExportOptions::default());
        let logins: Vec<_> = payload.logins.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(logins, ["l1"]);
        let keys: Vec<_> = payload.keys.iter().map(|k| k.id.as_str()).collect();
        assert_eq!(keys, ["k-b"]);
    }

    #[test]
    fn secrets_and_keys_only_when_asked() {
        let mut env = MapEnv::default();
        env.secrets.insert("pp-k-web".into(), b"pass".to_vec());
        env.secrets.insert("pp-k-b".into(), b"bpass".to_vec());
        env.files.insert("/keys/k-web".into(), b"KEY".to_vec());
        env.files.insert("/keys/k-b".into(), b"BKEY".to_vec());
        let (plain, _) = build(&data(), &env, &hosts_only(&["web"]), &ExportOptions::default());
        assert!(plain.secrets.is_empty() && plain.key_files.is_empty());

        let opts = ExportOptions { include_secrets: true, include_keys: true };
        let (full, warnings) = build(&data(), &env, &hosts_only(&["web"]), &opts);
        assert_eq!(full.secrets.get("pp-k-web").map(|v| v.as_str()), Some("pass"));
        let key = full.key_files.get("/keys/k-web").unwrap();
        assert_eq!(key.file_name, "k-web");
        assert_eq!(STANDARD.decode(key.data.as_bytes()).unwrap(), b"KEY");
        assert!(warnings.iter().all(|w| !w.starts_with("Couldn't")), "{warnings:?}");
    }

    #[test]
    fn app_managed_key_is_exported_under_its_original_name() {
        let id = "6f1c2a9e-8d0b-4c57-9a3e-2b7d5e41f0c8";
        assert_eq!(file_name(&format!("C:\\cfg\\keys\\{id}-id_ed25519")), "id_ed25519");
        assert_eq!(file_name(&format!("/cfg/keys/{id}-{id}-id")), format!("{id}-id"));
        assert_eq!(file_name(&format!("/cfg/keys/{id}-")), format!("{id}-"));
        assert_eq!(file_name(&format!("/cfg/keys/{id}")), id);
        assert_eq!(file_name("/home/me/.ssh/my-server-key"), "my-server-key");
        assert_eq!(file_name("/home/me/.ssh/"), "key");

        let mut d = data();
        d.keys[0].path = format!("/cfg/keys/{id}-id");
        let mut env = MapEnv::default();
        env.files.insert(format!("/cfg/keys/{id}-id"), b"KEY".to_vec());
        env.files.insert("/keys/k-b".into(), b"BKEY".to_vec());
        let opts = ExportOptions { include_secrets: false, include_keys: true };
        let (payload, _) = build(&d, &env, &hosts_only(&["web"]), &opts);
        assert_eq!(payload.key_files[&format!("/cfg/keys/{id}-id")].file_name, "id");
    }

    #[test]
    fn unreadable_key_and_binary_secret_become_warnings() {
        let mut env = MapEnv::default();
        env.secrets.insert("pp-k-web".into(), vec![0xff, 0xfe]);
        let opts = ExportOptions { include_secrets: true, include_keys: true };
        let (payload, warnings) = build(&data(), &env, &hosts_only(&["web"]), &opts);
        assert!(!payload.secrets.contains_key("pp-k-web"));
        assert!(payload.key_files.is_empty());
        assert!(warnings.iter().any(|w| w.contains("/keys/k-web")));
        assert!(warnings.iter().any(|w| w.contains("k-web") && w.contains("password")));
    }

    #[test]
    fn vnc_and_rdp_gateway_passwords_are_exported_only_when_asked() {
        let rdp = conn(
            "office-rdp",
            ConnectionKind::Rdp(RdpConn {
                port: 3389,
                login: own("faye", Some("rdp-pw")),
                domain: None,
                keyboard_layout: None,
                audio: true,
                shared_folder: None,
                gateway: Some(RdpGatewayConn {
                    host: "gw".into(),
                    port: 443,
                    login: Some(own("gw", Some("gw-pw"))),
                    extra: Default::default(),
                }),
                extra: Default::default(),
            }),
        );
        let mut d = data();
        d.hosts.push(host("office", "office", "10.0.0.9", vec![rdp, vnc("desk", Some("vnc-pw"))]));
        let mut env = MapEnv::default();
        env.secrets.insert("rdp-pw".into(), b"r".to_vec());
        env.secrets.insert("gw-pw".into(), b"g".to_vec());
        env.secrets.insert("vnc-pw".into(), b"v".to_vec());

        let (plain, warnings) = build(&d, &env, &hosts_only(&["office"]), &ExportOptions::default());
        assert!(plain.secrets.is_empty() && warnings.is_empty());

        let opts = ExportOptions { include_secrets: true, include_keys: false };
        let (full, warnings) = build(&d, &env, &hosts_only(&["office"]), &opts);
        let secrets: Vec<_> = full.secrets.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(secrets, [("gw-pw", "g"), ("rdp-pw", "r"), ("vnc-pw", "v")]);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn an_unreadable_vnc_password_names_its_host() {
        let mut d = data();
        d.hosts.push(host("desk", "desk", "10.0.0.5", vec![vnc("desk-vnc", Some("vnc-pw"))]));
        let opts = ExportOptions { include_secrets: true, include_keys: false };
        let (payload, warnings) = build(&d, &MapEnv::default(), &hosts_only(&["desk"]), &opts);
        assert!(payload.secrets.is_empty());
        assert_eq!(warnings, ["Couldn't read a saved password for \"desk\""]);
    }
}
