use std::collections::{BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use specta::Type;

use super::model::{AuthMethod, IconRef, ProfileKey, RdpGateway, SharedFolder, Tunnel};
use crate::error::{AppError, AppResult};

pub const THIS_COMPUTER: &str = "this-computer";

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HostMove {
    pub id: String,
    pub folder_id: Option<String>,
    pub order: u32,
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderMove {
    pub id: String,
    pub parent_id: Option<String>,
    pub order: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HostsFile {
    pub version: u32,
    #[serde(default)]
    pub folders: Vec<Folder>,
    #[serde(default)]
    pub hosts: Vec<Host>,
    #[serde(default)]
    pub logins: Vec<SavedLogin>,
    #[serde(default)]
    pub keys: Vec<SshKey>,
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub pending_deletes: Vec<PendingDelete>,
    // What a later version keeps in the file, written back as it was read.
    #[serde(flatten)]
    #[specta(skip)]
    pub unknown: Map<String, Value>,
}

impl HostsFile {
    pub const CURRENT_VERSION: u32 = 1;

    pub fn connections(&self) -> impl Iterator<Item = &Connection> {
        self.hosts.iter().flat_map(|h| &h.connections)
    }

    fn pending_hosts(&self) -> impl Iterator<Item = &Host> {
        self.pending_deletes.iter().filter_map(|p| match &p.item {
            DeletedItem::Host(d) => Some(&d.host),
            DeletedItem::Connection(_) | DeletedItem::Other(_) => None,
        })
    }

    fn pending_connections(&self) -> impl Iterator<Item = &Connection> {
        self.pending_deletes.iter().flat_map(|p| match &p.item {
            DeletedItem::Host(d) => d.host.connections.iter().collect::<Vec<_>>(),
            DeletedItem::Connection(d) => vec![&d.connection],
            DeletedItem::Other(_) => vec![],
        })
    }

    pub fn referenced_secrets(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut add = |id: &Option<String>| {
            if let Some(id) = id {
                out.insert(id.clone());
            }
        };
        for l in &self.logins {
            add(&l.secret_id);
        }
        for k in &self.keys {
            add(&k.secret_id);
        }
        // A pending delete still holds its secrets, so Undo gets them back.
        for c in self.connections().chain(self.pending_connections()) {
            for login in c.kind.logins() {
                if let Login::Own(own) = login {
                    add(&own.secret_id);
                }
            }
            match &c.kind {
                ConnectionKind::S3(s) => add(&s.secret_id),
                ConnectionKind::Vnc(v) => add(&v.secret_id),
                _ => {}
            }
        }
        out
    }

    pub fn custom_icons(&self) -> BTreeSet<String> {
        let custom = |icon: &IconRef| match icon {
            IconRef::Custom { path } => Some(path.clone()),
            IconRef::Builtin { .. } => None,
        };
        self.hosts
            .iter()
            .chain(self.pending_hosts())
            .filter_map(|h| custom(&h.icon))
            .chain(self.folders.iter().filter_map(|f| custom(&f.icon)))
            .collect()
    }

    pub fn key_paths(&self) -> Vec<String> {
        self.keys.iter().map(|k| k.path.clone()).collect()
    }

    // The tunnels of the hosts still there (not those waiting in pending deletes), each with the
    // connection it runs on.
    fn tunnels(&self) -> impl Iterator<Item = (&str, &Tunnel)> {
        self.hosts.iter().flat_map(|h| &h.connections).flat_map(|c| {
            let tunnels = match &c.kind {
                ConnectionKind::Ssh(s) => s.tunnels.as_slice(),
                _ => &[],
            };
            tunnels.iter().map(move |t| (c.id.as_str(), t))
        })
    }

    pub fn validate(&self) -> AppResult<()> {
        let invalid = |m: String| Err(AppError::Invalid(m));
        unique("host", self.hosts.iter().map(|h| &h.id))?;
        unique("connection", self.connections().map(|c| &c.id))?;
        unique("folder", self.folders.iter().map(|f| &f.id))?;
        unique("saved login", self.logins.iter().map(|l| &l.id))?;
        unique("key", self.keys.iter().map(|k| &k.id))?;
        unique("tag", self.tags.iter().map(|t| &t.id))?;
        unique("pending delete", self.pending_deletes.iter().map(|p| &p.id))?;

        let folders: HashMap<&str, Option<&str>> =
            self.folders.iter().map(|f| (f.id.as_str(), f.parent_id.as_deref())).collect();
        for f in &self.folders {
            let mut seen = HashSet::from([f.id.as_str()]);
            let mut parent = f.parent_id.as_deref();
            while let Some(p) = parent {
                let Some(next) = folders.get(p) else {
                    return invalid(format!("The folder {} has a parent that doesn't exist.", f.name));
                };
                if !seen.insert(p) {
                    return invalid(format!("The folder {} can't go inside its own subfolder.", f.name));
                }
                parent = *next;
            }
        }

        let tags: HashSet<&str> = self.tags.iter().map(|t| t.id.as_str()).collect();
        let logins: HashSet<&str> = self.logins.iter().map(|l| l.id.as_str()).collect();
        let keys: HashSet<&str> = self.keys.iter().map(|k| k.id.as_str()).collect();
        for l in &self.logins {
            if l.key_ids.iter().any(|k| !keys.contains(k.as_str())) {
                return invalid(format!("A key of the saved login {} doesn't exist.", l.name));
            }
        }
        for h in &self.hosts {
            if h.folder_id.as_deref().is_some_and(|f| !folders.contains_key(f)) {
                return invalid(format!("The folder of {} doesn't exist.", h.name));
            }
            if h.tag_ids.iter().any(|t| !tags.contains(t.as_str())) {
                return invalid(format!("A tag of {} doesn't exist.", h.name));
            }
            let this_computer = h.id == THIS_COMPUTER;
            if this_computer && (h.folder_id.is_some() || !h.tag_ids.is_empty()) {
                return invalid("This computer can't go in a folder or have tags.".into());
            }
            for c in &h.connections {
                let local = matches!(c.kind, ConnectionKind::Local(_));
                if local != this_computer {
                    return invalid(if this_computer {
                        "This computer holds only local shells.".into()
                    } else {
                        format!("Local shells belong to This computer, not to {}.", h.name)
                    });
                }
                for login in c.kind.logins() {
                    match login {
                        Login::Saved(s) if !logins.contains(s.login_id.as_str()) => {
                            return invalid(format!("A saved login used by {} doesn't exist.", h.name));
                        }
                        Login::Own(o) if o.key_ids.iter().any(|k| !keys.contains(k.as_str())) => {
                            return invalid(format!("A key used by {} doesn't exist.", h.name));
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }
}

fn unique<'a>(what: &str, ids: impl Iterator<Item = &'a String>) -> AppResult<()> {
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(AppError::Invalid(format!("Two of the saved items share the {what} id {id}.")));
        }
    }
    Ok(())
}

// What the connect commands need, resolved at connect time: a tab holds only the connection
// id, so a later edit is picked up on the next connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    pub id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    pub keys: Vec<ProfileKey>,
    pub secret_id: Option<String>,
    pub jump_host_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SftpTarget {
    ViaSsh(String),
    Own {
        host: String,
        port: u16,
        username: String,
        auth_method: AuthMethod,
        keys: Vec<ProfileKey>,
        secret_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RdpResolved {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub domain: Option<String>,
    pub secret_id: Option<String>,
    pub keyboard_layout: Option<u32>,
    pub audio: bool,
    pub shared_folder: Option<SharedFolder>,
    pub gateway: Option<RdpGateway>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Resolved {
    pub endpoint: String,
    pub port: Option<u16>,
    pub region: String,
    pub use_tls: bool,
    pub path_style: bool,
    pub access_key_id: String,
    pub secret_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FtpResolved {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub secret_id: Option<String>,
    pub secure: bool,
    pub allow_invalid_cert: bool,
    pub ignore_hostname: bool,
}

struct ResolvedLogin {
    username: String,
    auth_method: AuthMethod,
    keys: Vec<ProfileKey>,
    secret_id: Option<String>,
}

fn gone() -> AppError {
    AppError::NotFound("This connection no longer exists.".into())
}

fn address(host: &Host, c: &Connection) -> String {
    c.address.clone().unwrap_or_else(|| host.address.clone())
}

impl HostsFile {
    fn find(&self, id: &str) -> AppResult<(&Host, &Connection)> {
        self.hosts
            .iter()
            .find_map(|h| h.connections.iter().find(|c| c.id == id).map(|c| (h, c)))
            .ok_or_else(gone)
    }

    fn resolve(&self, login: &Login) -> AppResult<ResolvedLogin> {
        let (username, auth_method, key_ids, secret_id) = match login {
            Login::Own(o) => (&o.username, o.auth_method, &o.key_ids, &o.secret_id),
            Login::Saved(s) => {
                let l = self
                    .logins
                    .iter()
                    .find(|l| l.id == s.login_id)
                    .ok_or_else(|| AppError::Invalid("Its saved login no longer exists.".into()))?;
                (&l.username, l.auth_method, &l.key_ids, &l.secret_id)
            }
        };
        let keys = key_ids
            .iter()
            .map(|id| {
                self.keys
                    .iter()
                    .find(|k| &k.id == id)
                    .map(|k| ProfileKey { path: k.path.clone(), secret_id: k.secret_id.clone() })
                    .ok_or_else(|| AppError::Invalid("One of its keys no longer exists.".into()))
            })
            .collect::<AppResult<_>>()?;
        Ok(ResolvedLogin {
            username: username.clone(),
            auth_method,
            keys,
            secret_id: secret_id.clone(),
        })
    }

    pub fn ssh_target(&self, id: &str) -> AppResult<SshTarget> {
        let (host, c) = self.find(id)?;
        let ConnectionKind::Ssh(ssh) = &c.kind else { return Err(gone()) };
        let login = self.resolve(&ssh.login)?;
        Ok(SshTarget {
            id: c.id.clone(),
            host: address(host, c),
            port: ssh.port,
            username: login.username,
            auth_method: login.auth_method,
            keys: login.keys,
            secret_id: login.secret_id,
            jump_host_id: ssh.jump_host_id.clone(),
        })
    }

    pub fn sftp_target(&self, id: &str) -> AppResult<SftpTarget> {
        let (host, c) = self.find(id)?;
        let ConnectionKind::Sftp(sftp) = &c.kind else { return Err(gone()) };
        let Some(own) = &sftp.own else {
            return host
                .connections
                .iter()
                .find(|c| matches!(c.kind, ConnectionKind::Ssh(_)))
                .map(|ssh| SftpTarget::ViaSsh(ssh.id.clone()))
                .ok_or_else(|| {
                    AppError::Invalid(
                        "This host has no SSH connection. Set a login for Files in the host editor."
                            .into(),
                    )
                });
        };
        let login = self.resolve(&own.login)?;
        Ok(SftpTarget::Own {
            host: address(host, c),
            port: own.port,
            username: login.username,
            auth_method: login.auth_method,
            keys: login.keys,
            secret_id: login.secret_id,
        })
    }

    pub fn vnc_target(&self, id: &str) -> AppResult<(String, u16, Option<String>, Option<String>)> {
        let (host, c) = self.find(id)?;
        let ConnectionKind::Vnc(vnc) = &c.kind else { return Err(gone()) };
        Ok((address(host, c), vnc.port, vnc.username.clone(), vnc.secret_id.clone()))
    }

    pub fn rdp_target(&self, id: &str) -> AppResult<RdpResolved> {
        let (host, c) = self.find(id)?;
        let ConnectionKind::Rdp(rdp) = &c.kind else { return Err(gone()) };
        let login = self.resolve(&rdp.login)?;
        let gateway = match &rdp.gateway {
            None => None,
            Some(g) => Some(match &g.login {
                None => RdpGateway {
                    host: g.host.clone(),
                    port: g.port,
                    same_login: true,
                    username: String::new(),
                    secret_id: None,
                    extra: Default::default(),
                },
                Some(l) => {
                    let l = self.resolve(l)?;
                    RdpGateway {
                        host: g.host.clone(),
                        port: g.port,
                        same_login: false,
                        username: l.username,
                        secret_id: l.secret_id,
                        extra: Default::default(),
                    }
                }
            }),
        };
        Ok(RdpResolved {
            host: address(host, c),
            port: rdp.port,
            username: login.username,
            domain: rdp.domain.clone(),
            secret_id: login.secret_id,
            keyboard_layout: rdp.keyboard_layout,
            audio: rdp.audio,
            shared_folder: rdp.shared_folder.clone(),
            gateway,
        })
    }

    pub fn s3_target(&self, id: &str) -> AppResult<S3Resolved> {
        let (host, c) = self.find(id)?;
        let ConnectionKind::S3(s3) = &c.kind else { return Err(gone()) };
        Ok(S3Resolved {
            endpoint: address(host, c),
            port: s3.port,
            region: s3.region.clone(),
            use_tls: s3.use_tls,
            path_style: s3.path_style,
            access_key_id: s3.access_key_id.clone(),
            secret_id: s3.secret_id.clone(),
        })
    }

    pub fn ftp_target(&self, id: &str) -> AppResult<FtpResolved> {
        let (host, c) = self.find(id)?;
        let ConnectionKind::Ftp(ftp) = &c.kind else { return Err(gone()) };
        let login = self.resolve(&ftp.login)?;
        if login.auth_method != AuthMethod::Password {
            return Err(AppError::Invalid("FTP needs a password login.".into()));
        }
        Ok(FtpResolved {
            host: address(host, c),
            port: ftp.port,
            username: login.username,
            secret_id: login.secret_id,
            secure: ftp.secure,
            allow_invalid_cert: ftp.allow_invalid_cert,
            ignore_hostname: ftp.ignore_hostname,
        })
    }
}

impl HostsFile {
    // Every connection with its host's name, the pending deletes included: Undo brings them
    // back, so what they use must stay.
    fn named_connections(&self) -> Vec<(String, &Connection)> {
        let host_name = |id: &str| self.hosts.iter().find(|h| h.id == id).map(|h| h.name.clone());
        let mut out: Vec<(String, &Connection)> = self
            .hosts
            .iter()
            .flat_map(|h| h.connections.iter().map(|c| (h.name.clone(), c)))
            .collect();
        for p in &self.pending_deletes {
            match &p.item {
                DeletedItem::Host(d) => {
                    out.extend(d.host.connections.iter().map(|c| (d.host.name.clone(), c)))
                }
                DeletedItem::Connection(d) => {
                    out.push((host_name(&d.host_id).unwrap_or_default(), &d.connection))
                }
                DeletedItem::Other(_) => {}
            }
        }
        out
    }

    // "host · connection" for each connection signing in with the saved login.
    pub fn login_users(&self, login_id: &str) -> Vec<String> {
        let uses = |l: &Login| matches!(l, Login::Saved(s) if s.login_id == login_id);
        let mut out = Vec::new();
        for (host, c) in self.named_connections() {
            let name = format!("{host} · {}", c.label.as_deref().unwrap_or(c.kind.type_name()));
            if let ConnectionKind::Rdp(r) = &c.kind {
                if uses(&r.login) {
                    out.push(name.clone());
                }
                if r.gateway.as_ref().and_then(|g| g.login.as_ref()).is_some_and(uses) {
                    out.push(format!("{name} gateway"));
                }
            } else if c.kind.logins().into_iter().any(uses) {
                out.push(name);
            }
        }
        out
    }

    // Saved logins and connections that sign in with the key.
    pub fn key_users(&self, key_id: &str) -> Vec<String> {
        let has = |ids: &[String]| ids.iter().any(|k| k == key_id);
        let mut out: Vec<String> = self
            .logins
            .iter()
            .filter(|l| has(&l.key_ids))
            .map(|l| format!("the login {}", l.name))
            .collect();
        for (host, c) in self.named_connections() {
            if c.kind.logins().into_iter().any(|l| matches!(l, Login::Own(o) if has(&o.key_ids))) {
                out.push(format!("{host} · {}", c.label.as_deref().unwrap_or(c.kind.type_name())));
            }
        }
        out
    }
}

impl ConnectionKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            ConnectionKind::Ssh(_) => "SSH",
            ConnectionKind::Sftp(_) => "Files",
            ConnectionKind::Ftp(_) => "FTP",
            ConnectionKind::S3(_) => "Storage",
            ConnectionKind::Rdp(_) => "RDP",
            ConnectionKind::Vnc(_) => "VNC",
            ConnectionKind::Local(_) => "Local shell",
        }
    }

    pub fn logins(&self) -> Vec<&Login> {
        match self {
            ConnectionKind::Ssh(s) => vec![&s.login],
            ConnectionKind::Sftp(s) => s.own.iter().map(|o| &o.login).collect(),
            ConnectionKind::Ftp(f) => vec![&f.login],
            ConnectionKind::Rdp(r) => {
                let mut out = vec![&r.login];
                out.extend(r.gateway.as_ref().and_then(|g| g.login.as_ref()));
                out
            }
            ConnectionKind::S3(_) | ConnectionKind::Vnc(_) | ConnectionKind::Local(_) => vec![],
        }
    }
}

// A running tunnel whose forward was removed, changed or moved to another connection must stop:
// it would keep forwarding what the user took away, with no row left to stop it from.
pub fn changed_tunnels(before: &HostsFile, after: &HostsFile) -> Vec<String> {
    let forward = |(conn, t): (&str, &Tunnel)| (conn.to_string(), Tunnel { auto_start: false, ..t.clone() });
    let now: HashMap<&str, _> = after.tunnels().map(|x| (x.1.id.as_str(), forward(x))).collect();
    before
        .tunnels()
        .filter(|&x| now.get(x.1.id.as_str()) != Some(&forward(x)))
        .map(|(_, t)| t.id.clone())
        .collect()
}

// A deleted host or connection, kept until its Undo notice closes or the next start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PendingDelete {
    pub id: String,
    // Unix time in milliseconds. The override only names it a plain number for TypeScript; the
    // value stays well inside a JS number.
    #[specta(type = u32)]
    pub deleted_at: u64,
    pub item: DeletedItem,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DeletedItem {
    Host(DeletedHost),
    Connection(DeletedConnection),
    // A kind a later version deletes: kept as written and purged with the rest, since failing
    // to read it would set the whole file aside.
    #[serde(untagged)]
    #[specta(skip)]
    Other(Value),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeletedHost {
    pub host: Host,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeletedConnection {
    pub host_id: String,
    pub index: u32,
    pub connection: Connection,
}

// A connection the host editor moves elsewhere, saved in the same write as the rest of its edits.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionMove {
    pub connection: Connection,
    pub to: MoveTarget,
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MoveTarget {
    Host(MoveToHost),
    NewHost(NewHostTarget),
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MoveToHost {
    pub host_id: String,
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NewHostTarget {
    pub name: String,
    pub address: String,
    pub folder_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub icon: IconRef,
    pub order: u32,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub id: String,
    pub name: String,
    pub address: String,
    pub folder_id: Option<String>,
    #[serde(default)]
    pub tag_ids: Vec<String>,
    #[serde(default)]
    pub icon: IconRef,
    pub color: Option<String>,
    pub order: u32,
    pub connections: Vec<Connection>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub label: Option<String>,
    pub address: Option<String>,
    pub kind: ConnectionKind,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConnectionKind {
    Ssh(SshConn),
    Sftp(SftpConn),
    Ftp(FtpConn),
    S3(S3Conn),
    Rdp(RdpConn),
    Vnc(VncConn),
    Local(LocalConn),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SshConn {
    pub port: u16,
    pub login: Login,
    pub jump_host_id: Option<String>,
    #[serde(default)]
    pub tunnels: Vec<Tunnel>,
    #[serde(default)]
    pub appearance: Option<ConnAppearance>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnAppearance {
    pub colors: Option<String>,
    pub font_family: Option<String>,
    pub font_size: Option<u16>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

// `own: None` is "same login as SSH": the host's first SSH connection is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SftpConn {
    pub own: Option<OwnSftp>,
    pub start_folder: Option<String>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OwnSftp {
    pub port: u16,
    pub login: Login,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FtpConn {
    pub port: u16,
    pub login: Login,
    pub secure: bool,
    pub allow_invalid_cert: bool,
    pub ignore_hostname: bool,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct S3Conn {
    pub port: Option<u16>,
    pub region: String,
    pub use_tls: bool,
    pub path_style: bool,
    pub access_key_id: String,
    pub secret_id: Option<String>,
    pub bucket: Option<String>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RdpConn {
    pub port: u16,
    pub login: Login,
    pub domain: Option<String>,
    pub keyboard_layout: Option<u32>,
    #[serde(default = "default_true")]
    pub audio: bool,
    #[serde(default)]
    pub shared_folder: Option<SharedFolder>,
    #[serde(default)]
    pub gateway: Option<RdpGatewayConn>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

// `login: None` gives the gateway the RDP login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RdpGatewayConn {
    pub host: String,
    pub port: u16,
    pub login: Option<Login>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VncConn {
    pub port: u16,
    pub username: Option<String>,
    pub secret_id: Option<String>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LocalConn {
    pub program: String,
    pub start_folder: Option<String>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Login {
    Saved(SavedLoginRef),
    Own(OwnLogin),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedLoginRef {
    pub login_id: String,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OwnLogin {
    pub username: String,
    pub auth_method: AuthMethod,
    #[serde(default)]
    pub key_ids: Vec<String>,
    pub secret_id: Option<String>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedLogin {
    pub id: String,
    pub name: String,
    pub username: String,
    pub auth_method: AuthMethod,
    #[serde(default)]
    pub key_ids: Vec<String>,
    pub secret_id: Option<String>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

// One key file, listed once, with its passphrase (a vault reference).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SshKey {
    pub id: String,
    pub path: String,
    pub secret_id: Option<String>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
    pub show_color: bool,
    pub ask_before_close: bool,
    pub ask_before_paste: bool,
    pub terminal_colors: Option<String>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: Map<String, Value>,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub fn own(user: &str, secret: Option<&str>) -> Login {
        Login::Own(OwnLogin {
            username: user.into(),
            auth_method: AuthMethod::Password,
            key_ids: vec![],
            secret_id: secret.map(Into::into),
            extra: Default::default(),
        })
    }

    pub fn conn(id: &str, kind: ConnectionKind) -> Connection {
        Connection { id: id.into(), label: None, address: None, kind, extra: Default::default() }
    }

    pub fn ssh(id: &str, port: u16, login: Login) -> Connection {
        conn(
            id,
            ConnectionKind::Ssh(SshConn {
                port,
                login,
                jump_host_id: None,
                tunnels: vec![],
                appearance: None,
                extra: Default::default(),
            }),
        )
    }

    pub fn vnc(id: &str, secret: Option<&str>) -> Connection {
        conn(
            id,
            ConnectionKind::Vnc(VncConn {
                port: 5900,
                username: None,
                secret_id: secret.map(Into::into),
                extra: Default::default(),
            }),
        )
    }

    pub fn local(id: &str, program: &str) -> Connection {
        conn(
            id,
            ConnectionKind::Local(LocalConn {
                program: program.into(),
                start_folder: None,
                extra: Default::default(),
            }),
        )
    }

    pub fn host(id: &str, name: &str, address: &str, connections: Vec<Connection>) -> Host {
        Host {
            id: id.into(),
            name: name.into(),
            address: address.into(),
            folder_id: None,
            tag_ids: vec![],
            icon: IconRef::default(),
            color: None,
            order: 0,
            connections,
            extra: Default::default(),
        }
    }

    pub fn folder(id: &str, parent: Option<&str>) -> Folder {
        Folder {
            id: id.into(),
            name: id.into(),
            parent_id: parent.map(Into::into),
            icon: IconRef::default(),
            order: 0,
            extra: Default::default(),
        }
    }

    pub fn file(hosts: Vec<Host>) -> HostsFile {
        HostsFile { version: HostsFile::CURRENT_VERSION, hosts, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn a_removed_or_changed_tunnel_is_stopped() {
        use crate::store::model::TunnelKind;
        let tunnel = |id: &str, port: u16| Tunnel {
            id: id.into(),
            kind: TunnelKind::Local,
            bind_host: "127.0.0.1".into(),
            bind_port: port,
            target_host: Some("db".into()),
            target_port: Some(5432),
            auto_start: false,
        };
        let with = |id: &str, tunnels: Vec<Tunnel>| {
            let mut c = ssh(id, 22, own("u", None));
            if let ConnectionKind::Ssh(s) = &mut c.kind {
                s.tunnels = tunnels;
            }
            c
        };
        let before = file(vec![
            host("h1", "a", "a", vec![
                with("c1", vec![tunnel("kept", 1), tunnel("removed", 2), tunnel("rebound", 3), tunnel("auto", 4), tunnel("moved", 5)]),
                with("c2", vec![]),
            ]),
            host("h2", "b", "b", vec![with("c3", vec![tunnel("host-deleted", 6)])]),
        ]);
        let after = file(vec![host("h1", "a", "a", vec![
            with("c1", vec![tunnel("kept", 1), tunnel("rebound", 30), Tunnel { auto_start: true, ..tunnel("auto", 4) }]),
            with("c2", vec![tunnel("moved", 5)]),
        ])]);

        let mut stopped = changed_tunnels(&before, &after);
        stopped.sort();
        assert_eq!(stopped, ["host-deleted", "moved", "rebound", "removed"]);
        assert!(changed_tunnels(&before, &before).is_empty());
    }

    #[test]
    fn referenced_secrets_covers_every_kind() {
        let saved = Login::Saved(SavedLoginRef { login_id: "l1".into(), extra: Default::default() });
        let mut f = file(vec![host(
            "h",
            "h",
            "h",
            vec![
                ssh("ssh", 22, own("u", Some("ssh-pw"))),
                conn(
                    "sftp",
                    ConnectionKind::Sftp(SftpConn {
                        own: Some(OwnSftp { port: 22, login: own("u", Some("sftp-pw")), extra: Default::default() }),
                        start_folder: None,
                        extra: Default::default(),
                    }),
                ),
                conn(
                    "ftp",
                    ConnectionKind::Ftp(FtpConn {
                        port: 21,
                        login: own("u", Some("ftp-pw")),
                        secure: false,
                        allow_invalid_cert: false,
                        ignore_hostname: false,
                        extra: Default::default(),
                    }),
                ),
                conn(
                    "s3",
                    ConnectionKind::S3(S3Conn {
                        port: None,
                        region: "r".into(),
                        use_tls: true,
                        path_style: false,
                        access_key_id: "ak".into(),
                        secret_id: Some("s3-secret".into()),
                        bucket: None,
                        extra: Default::default(),
                    }),
                ),
                conn(
                    "rdp",
                    ConnectionKind::Rdp(RdpConn {
                        port: 3389,
                        login: own("u", Some("rdp-pw")),
                        domain: None,
                        keyboard_layout: None,
                        audio: true,
                        shared_folder: None,
                        gateway: Some(RdpGatewayConn {
                            host: "gw".into(),
                            port: 443,
                            login: Some(own("g", Some("gw-pw"))),
                            extra: Default::default(),
                        }),
                        extra: Default::default(),
                    }),
                ),
                vnc("vnc", Some("vnc-pw")),
                ssh("saved", 22, saved),
            ],
        )]);
        f.logins.push(SavedLogin {
            id: "l1".into(),
            name: "n".into(),
            username: "u".into(),
            auth_method: AuthMethod::Password,
            key_ids: vec![],
            secret_id: Some("login-pw".into()),
            extra: Default::default(),
        });
        f.keys.push(SshKey {
            id: "k1".into(),
            path: "/k".into(),
            secret_id: Some("key-pass".into()),
            extra: Default::default(),
        });
        let expected: BTreeSet<String> = [
            "ssh-pw", "sftp-pw", "ftp-pw", "s3-secret", "rdp-pw", "gw-pw", "vnc-pw", "login-pw",
            "key-pass",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        assert_eq!(f.referenced_secrets(), expected);
    }

    fn sftp_same_login(id: &str) -> Connection {
        conn(
            id,
            ConnectionKind::Sftp(SftpConn { own: None, start_folder: None, extra: Default::default() }),
        )
    }

    fn rdp_with_gateway(id: &str, gateway_login: Option<Login>) -> Connection {
        conn(
            id,
            ConnectionKind::Rdp(RdpConn {
                port: 3389,
                login: own("faye", Some("rdp-pw")),
                domain: Some("corp".into()),
                keyboard_layout: Some(0x407),
                audio: false,
                shared_folder: None,
                gateway: Some(RdpGatewayConn {
                    host: "gw.corp".into(),
                    port: 443,
                    login: gateway_login,
                    extra: Default::default(),
                }),
                extra: Default::default(),
            }),
        )
    }

    #[test]
    fn ssh_target_resolves_a_saved_login_and_its_keys() {
        let saved = Login::Saved(SavedLoginRef { login_id: "l1".into(), extra: Default::default() });
        let mut f = file(vec![host("h", "box", "10.0.0.1", vec![ssh("c", 2222, saved)])]);
        f.logins.push(SavedLogin {
            id: "l1".into(),
            name: "homelab".into(),
            username: "admin".into(),
            auth_method: AuthMethod::Key,
            key_ids: vec!["k1".into()],
            secret_id: None,
            extra: Default::default(),
        });
        f.keys.push(SshKey {
            id: "k1".into(),
            path: "/k".into(),
            secret_id: Some("p".into()),
            extra: Default::default(),
        });
        let t = f.ssh_target("c").unwrap();
        assert_eq!((t.host.as_str(), t.port), ("10.0.0.1", 2222));
        assert_eq!(t.username, "admin");
        assert_eq!(t.auth_method, AuthMethod::Key);
        assert_eq!(t.keys, [ProfileKey { path: "/k".into(), secret_id: Some("p".into()) }]);
    }

    #[test]
    fn ftp_target_resolves_a_saved_login() {
        let login = |method| SavedLogin {
            id: "l1".into(),
            name: "media".into(),
            username: "media".into(),
            auth_method: method,
            key_ids: vec![],
            secret_id: Some("pw".into()),
            extra: Default::default(),
        };
        let ftp = conn(
            "c",
            ConnectionKind::Ftp(FtpConn {
                port: 21,
                login: Login::Saved(SavedLoginRef { login_id: "l1".into(), extra: Default::default() }),
                secure: true,
                allow_invalid_cert: false,
                ignore_hostname: false,
                extra: Default::default(),
            }),
        );
        let mut f = file(vec![host("h", "nas", "nas.local", vec![ftp])]);
        f.logins.push(login(AuthMethod::Password));
        let t = f.ftp_target("c").unwrap();
        assert_eq!((t.host.as_str(), t.port, t.username.as_str()), ("nas.local", 21, "media"));
        assert_eq!((t.secret_id.as_deref(), t.secure), (Some("pw"), true));

        f.logins[0] = login(AuthMethod::Key);
        assert!(matches!(f.ftp_target("c"), Err(AppError::Invalid(m)) if m == "FTP needs a password login."));
    }

    #[test]
    fn connection_address_overrides_the_host_address() {
        let mut c = ssh("c", 22, own("u", None));
        c.address = Some("10.9.9.9".into());
        let f = file(vec![host("h", "box", "10.0.0.1", vec![c])]);
        assert_eq!(f.ssh_target("c").unwrap().host, "10.9.9.9");
    }

    #[test]
    fn an_unknown_connection_no_longer_exists() {
        let f = file(vec![]);
        assert!(matches!(
            f.ssh_target("nope"),
            Err(AppError::NotFound(m)) if m == "This connection no longer exists."
        ));
    }

    #[test]
    fn sftp_same_login_uses_the_first_ssh_connection() {
        let f = file(vec![host(
            "h",
            "box",
            "a",
            vec![sftp_same_login("f"), ssh("s1", 22, own("u", None)), ssh("s2", 22, own("r", None))],
        )]);
        assert_eq!(f.sftp_target("f").unwrap(), SftpTarget::ViaSsh("s1".into()));
    }

    #[test]
    fn sftp_same_login_without_ssh_says_what_to_do() {
        let f = file(vec![host("h", "box", "a", vec![sftp_same_login("f")])]);
        assert!(matches!(
            f.sftp_target("f"),
            Err(AppError::Invalid(m))
                if m == "This host has no SSH connection. Set a login for Files in the host editor."
        ));
    }

    #[test]
    fn rdp_gateway_without_login_is_same_login() {
        let f = file(vec![host(
            "h",
            "office",
            "10.0.0.9",
            vec![
                rdp_with_gateway("same", None),
                rdp_with_gateway("own", Some(own("gw-faye", Some("gw-pw")))),
            ],
        )]);
        let same = f.rdp_target("same").unwrap();
        assert_eq!(same.username, "faye");
        assert_eq!(same.secret_id.as_deref(), Some("rdp-pw"));
        assert!(same.gateway.as_ref().unwrap().same_login);
        let own = f.rdp_target("own").unwrap().gateway.unwrap();
        assert!(!own.same_login);
        assert_eq!(own.username, "gw-faye");
        assert_eq!(own.secret_id.as_deref(), Some("gw-pw"));
        assert_eq!((own.host.as_str(), own.port), ("gw.corp", 443));
    }

    #[test]
    fn validate_rejects_a_local_connection_on_another_host() {
        let f = file(vec![host("h", "h", "h", vec![local("c", "cmd.exe")])]);
        assert!(matches!(f.validate(), Err(AppError::Invalid(_))));
        let ok = file(vec![host(THIS_COMPUTER, "This computer", "", vec![local("c", "cmd.exe")])]);
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn validate_rejects_a_missing_saved_login_or_key() {
        let saved = Login::Saved(SavedLoginRef { login_id: "nope".into(), extra: Default::default() });
        let f = file(vec![host("h", "h", "h", vec![ssh("c", 22, saved)])]);
        assert!(matches!(f.validate(), Err(AppError::Invalid(_))));
        let keyed = Login::Own(OwnLogin {
            username: "u".into(),
            auth_method: AuthMethod::Key,
            key_ids: vec!["gone".into()],
            secret_id: None,
            extra: Default::default(),
        });
        let f = file(vec![host("h", "h", "h", vec![ssh("c", 22, keyed)])]);
        assert!(matches!(f.validate(), Err(AppError::Invalid(_))));
    }

    #[test]
    fn validate_rejects_duplicate_connection_ids_across_hosts() {
        let f = file(vec![
            host("a", "a", "a", vec![vnc("same", None)]),
            host("b", "b", "b", vec![vnc("same", None)]),
        ]);
        assert!(matches!(f.validate(), Err(AppError::Invalid(_))));
    }

    #[test]
    fn connection_kind_is_tagged_by_type() {
        let c = Connection {
            id: "c1".into(),
            label: None,
            address: None,
            kind: ConnectionKind::Vnc(VncConn {
                port: 5901,
                username: None,
                secret_id: Some("v".into()),
                extra: Default::default(),
            }),
            extra: Default::default(),
        };
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["kind"]["type"], "vnc");
        assert_eq!(json["kind"]["port"], 5901);
        assert_eq!(json["kind"]["secretId"], "v");
        assert_eq!(serde_json::from_value::<Connection>(json).unwrap(), c);
    }

    #[test]
    fn login_is_tagged_by_kind() {
        let saved =
            serde_json::to_value(Login::Saved(SavedLoginRef { login_id: "l1".into(), extra: Default::default() })).unwrap();
        assert_eq!(saved, serde_json::json!({ "kind": "saved", "loginId": "l1" }));
        let own: Login = serde_json::from_value(serde_json::json!(
            { "kind": "own", "username": "u", "authMethod": "key", "keyIds": ["k"], "secretId": null }
        ))
        .unwrap();
        assert!(matches!(own, Login::Own(OwnLogin { ref key_ids, .. }) if key_ids == &["k"]));
    }

    #[test]
    fn unknown_fields_round_trip_at_every_level() {
        let json = serde_json::json!({ "version": 1, "later": [1], "folders": [], "logins": [],
            "keys": [], "tags": [], "hosts": [{ "id": "h", "name": "n", "address": "a",
            "folderId": null, "color": null, "order": 0, "pinned": true,
            "connections": [{ "id": "c", "label": null, "address": null, "note": "x",
                "kind": { "type": "local", "program": "cmd.exe", "startFolder": null, "args": [] } }] }] });
        let file: HostsFile = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(file.unknown["later"], serde_json::json!([1]));
        assert_eq!(file.hosts[0].extra["pinned"], true);
        assert_eq!(file.hosts[0].connections[0].extra["note"], "x");
        let back = serde_json::to_value(&file).unwrap();
        assert_eq!(back["hosts"][0]["connections"][0]["kind"]["args"], serde_json::json!([]));
    }

    #[test]
    fn rdp_without_audio_or_gateway_fields_plays_sound() {
        let kind: ConnectionKind = serde_json::from_value(serde_json::json!({
            "type": "rdp", "port": 3389,
            "login": { "kind": "own", "username": "u", "authMethod": "password", "secretId": null },
            "domain": null, "keyboardLayout": null
        }))
        .unwrap();
        let ConnectionKind::Rdp(rdp) = kind else { panic!("not rdp") };
        assert!(rdp.audio);
        assert_eq!(rdp.shared_folder, None);
        assert_eq!(rdp.gateway, None);
    }
}
