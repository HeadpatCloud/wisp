use std::collections::HashMap;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use sha2::{Digest, Sha256};

use super::bundle::{LegacyPayload, PayloadV3};
use super::{Env, FieldDiff, ItemKind, ItemStatus, LocalMatch, ReviewItem};
use crate::net::address_key;
use crate::store::hosts::{
    ConnAppearance, Connection, ConnectionKind, Folder, Host, HostsFile, Login, RdpGatewayConn,
    SavedLogin, SshKey, Tag, THIS_COMPUTER,
};
use crate::store::migrate::migrate;
use crate::store::model::{AuthMethod, IconRef, ProfileStore, SharedFolder, Tunnel, TunnelKind};

pub struct Plan {
    pub items: Vec<ReviewItem>,
    pub unchanged: u32,
    pub matches: HashMap<String, String>,
}

// The ids of the review rows. The frontend sends them back in the decisions and `stage` applies
// a row by its id, so they must not change.
pub mod field {
    pub const NAME: &str = "name";
    pub const PARENT_ID: &str = "parentId";
    pub const ICON: &str = "icon";
    pub const ADDRESS: &str = "address";
    pub const FOLDER_ID: &str = "folderId";
    pub const TAGS: &str = "tags";
    pub const COLOR: &str = "color";
    pub const LABEL: &str = "label";
    pub const PORT: &str = "port";
    pub const LOGIN: &str = "login";
    pub const PASSWORD: &str = "password";
    pub const JUMP_HOST_ID: &str = "jumpHostId";
    pub const TUNNELS: &str = "tunnels";
    pub const APPEARANCE: &str = "appearance";
    pub const START_FOLDER: &str = "startFolder";
    pub const SFTP_LOGIN: &str = "sftpLogin";
    pub const SECURE: &str = "secure";
    pub const ALLOW_INVALID_CERT: &str = "allowInvalidCert";
    pub const IGNORE_HOSTNAME: &str = "ignoreHostname";
    pub const REGION: &str = "region";
    pub const USE_TLS: &str = "useTls";
    pub const PATH_STYLE: &str = "pathStyle";
    pub const ACCESS_KEY_ID: &str = "accessKeyId";
    pub const BUCKET: &str = "bucket";
    pub const DOMAIN: &str = "domain";
    pub const KEYBOARD_LAYOUT: &str = "keyboardLayout";
    pub const AUDIO: &str = "audio";
    pub const SHARED_FOLDER: &str = "sharedFolder";
    pub const GATEWAY: &str = "gateway";
    pub const GATEWAY_PASSWORD: &str = "gatewayPassword";
    pub const USERNAME: &str = "username";
    pub const AUTH_METHOD: &str = "authMethod";
    pub const KEYS: &str = "keys";
    pub const KEY_FILE: &str = "keyFile";
    pub const PASSPHRASE: &str = "passphrase";
    pub const SHOW_COLOR: &str = "showColor";
    pub const ASK_BEFORE_CLOSE: &str = "askBeforeClose";
    pub const ASK_BEFORE_PASTE: &str = "askBeforePaste";
    pub const TERMINAL_COLORS: &str = "terminalColors";
}

// The same list as src/lib/rdpLayouts.ts; a test keeps them equal.
pub(crate) const RDP_LAYOUTS: &[(u32, &str)] = &[
    (0x409, "English (US)"),
    (0x20409, "English (US International)"),
    (0x809, "English (UK)"),
    (0x407, "German"),
    (0x807, "German (Switzerland)"),
    (0x40c, "French"),
    (0x80c, "French (Belgium)"),
    (0x100c, "French (Switzerland)"),
    (0x413, "Dutch"),
    (0x813, "Belgian (Period)"),
    (0x40a, "Spanish"),
    (0x410, "Italian"),
    (0x816, "Portuguese"),
    (0x416, "Portuguese (Brazil)"),
    (0x406, "Danish"),
    (0x414, "Norwegian"),
    (0x41d, "Swedish"),
    (0x40b, "Finnish"),
    (0x415, "Polish (Programmers)"),
    (0x405, "Czech"),
    (0x40e, "Hungarian"),
    (0x41f, "Turkish Q"),
    (0x419, "Russian"),
    (0x411, "Japanese"),
];

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn folder_depth(f: &Folder, all: &[Folder]) -> usize {
    let mut depth = 0;
    let mut parent = f.parent_id.as_deref();
    while let Some(p) = parent {
        depth += 1;
        if depth > all.len() {
            break;
        }
        parent = all.iter().find(|x| x.id == p).and_then(|x| x.parent_id.as_deref());
    }
    depth
}

pub fn same_tunnel(a: &Tunnel, b: &Tunnel) -> bool {
    a.kind == b.kind
        && a.bind_host == b.bind_host
        && a.bind_port == b.bind_port
        && a.target_host == b.target_host
        && a.target_port == b.target_port
        && a.auto_start == b.auto_start
}

// A version 1 or 2 bundle, read through the same migration as a 0.5.0 profiles file. No shells:
// This computer is never imported.
pub fn from_legacy(p: LegacyPayload) -> PayloadV3 {
    let old = ProfileStore {
        version: 1,
        groups: p.groups,
        profiles: p.profiles,
        sftp_profiles: p.sftp_profiles,
        s3_profiles: p.s3_profiles,
        vnc_profiles: p.vnc_profiles,
        rdp_profiles: p.rdp_profiles,
        unknown: Default::default(),
    };
    let (file, _) = migrate(&old, &[]);
    PayloadV3 {
        folders: file.folders,
        hosts: file.hosts,
        logins: file.logins,
        keys: file.keys,
        tags: file.tags,
        secrets: p.secrets,
        key_files: p.key_files,
        legacy: true,
    }
}

// Matches in passes so an earlier look-alike can't take the local item a later one matches
// outright: first by id, then the only free local on the same server with the same name, then
// the only free local on the same server. The keys are (id, name); `same_server` also gets the
// incoming id -> local id pairs matched so far. `same_kind` rules out an id match across kinds.
fn match_items<'a, L, I>(
    locals: &'a [L],
    incoming: &[I],
    local_key: impl Fn(&L) -> (&str, &str),
    incoming_key: impl Fn(&I) -> (&str, &str),
    same_kind: impl Fn(&L, &I) -> bool,
    same_server: impl Fn(&L, &I, &HashMap<String, String>) -> bool,
) -> (Vec<Option<&'a L>>, HashMap<String, String>) {
    let mut found = vec![None; incoming.len()];
    let mut taken = vec![false; locals.len()];
    let mut map = HashMap::new();
    for (slot, inc) in found.iter_mut().zip(incoming) {
        let (id, _) = incoming_key(inc);
        let at = (0..locals.len()).find(|&at| {
            !taken[at] && local_key(&locals[at]).0 == id && same_kind(&locals[at], inc)
        });
        if let Some(at) = at {
            taken[at] = true;
            map.insert(id.to_string(), id.to_string());
            *slot = Some(&locals[at]);
        }
    }
    for by_name in [true, false] {
        for (slot, inc) in found.iter_mut().zip(incoming) {
            if slot.is_some() {
                continue;
            }
            let (id, name) = incoming_key(inc);
            let mut candidates = (0..locals.len()).filter(|&at| {
                !taken[at]
                    && same_server(&locals[at], inc, &map)
                    && (!by_name || local_key(&locals[at]).1 == name)
            });
            if let (Some(at), None) = (candidates.next(), candidates.next()) {
                taken[at] = true;
                map.insert(id.to_string(), local_key(&locals[at]).0.to_string());
                *slot = Some(&locals[at]);
            }
        }
    }
    (found, map)
}

fn push(fields: &mut Vec<FieldDiff>, field: &str, label: &str, local: String, incoming: String) {
    if local != incoming {
        fields.push(FieldDiff { field: field.into(), label: label.into(), local, incoming });
    }
}

fn text(v: Option<&str>) -> String {
    v.filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| "(none)".into())
}

fn on_off(on: bool) -> String {
    if on { "On" } else { "Off" }.into()
}

fn layout_label(layout: Option<u32>) -> String {
    match layout {
        None => "Automatic".into(),
        Some(id) => match RDP_LAYOUTS.iter().find(|(known, _)| *known == id) {
            Some((_, name)) => (*name).into(),
            None => format!("0x{id:x}"),
        },
    }
}

fn folder_label(folder: Option<&SharedFolder>) -> String {
    match folder {
        None => "None".into(),
        Some(f) => format!("{} as {}", f.path, f.name),
    }
}

fn auth(a: AuthMethod) -> String {
    match a {
        AuthMethod::Password => "password",
        AuthMethod::Key => "key",
        AuthMethod::Agent => "agent",
    }
    .into()
}

fn icon_label(i: &IconRef) -> String {
    match i {
        IconRef::Builtin { name } => name.clone(),
        IconRef::Custom { .. } => "custom image".into(),
    }
}

fn appearance_label(a: &Option<ConnAppearance>) -> String {
    match a {
        None => "(default)".into(),
        Some(a) => format!(
            "colours {}, font {} {}",
            a.colors.as_deref().unwrap_or("default"),
            a.font_family.as_deref().unwrap_or("default"),
            a.font_size.map(|s| s.to_string()).unwrap_or_else(|| "default".into()),
        ),
    }
}

fn tunnels_label(ts: &[Tunnel]) -> String {
    if ts.is_empty() {
        return "(none)".into();
    }
    ts.iter()
        .map(|t| {
            let kind = match t.kind {
                TunnelKind::Local => "L",
                TunnelKind::Remote => "R",
                TunnelKind::Dynamic => "D",
            };
            let route = match (&t.target_host, t.target_port) {
                (Some(h), Some(p)) => format!("{kind} {}:{} -> {h}:{p}", t.bind_host, t.bind_port),
                _ => format!("{kind} {}:{}", t.bind_host, t.bind_port),
            };
            if t.auto_start {
                format!("{route} (auto-start)")
            } else {
                route
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or(path)
}

pub fn kind_name(kind: &ConnectionKind) -> &'static str {
    match kind {
        ConnectionKind::Ssh(_) => "SSH",
        ConnectionKind::Sftp(_) => "Files",
        ConnectionKind::Ftp(_) => "FTP",
        ConnectionKind::S3(_) => "S3",
        ConnectionKind::Rdp(_) => "RDP",
        ConnectionKind::Vnc(_) => "VNC",
        ConnectionKind::Local(_) => "Local shell",
    }
}

fn same_type(a: &ConnectionKind, b: &ConnectionKind) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

fn login_user<'a>(login: &'a Login, logins: &'a [SavedLogin]) -> &'a str {
    match login {
        Login::Own(o) => &o.username,
        Login::Saved(s) => {
            logins.iter().find(|l| l.id == s.login_id).map(|l| l.username.as_str()).unwrap_or("")
        }
    }
}

// The port and user a connection is told apart by on a shared host.
fn port_and_user<'a>(kind: &'a ConnectionKind, logins: &'a [SavedLogin]) -> (u16, &'a str) {
    match kind {
        ConnectionKind::Ssh(c) => (c.port, login_user(&c.login, logins)),
        ConnectionKind::Sftp(c) => {
            c.own.as_ref().map(|o| (o.port, login_user(&o.login, logins))).unwrap_or((0, ""))
        }
        ConnectionKind::Ftp(c) => (c.port, login_user(&c.login, logins)),
        ConnectionKind::S3(c) => (c.port.unwrap_or(0), &c.access_key_id),
        ConnectionKind::Rdp(c) => (c.port, login_user(&c.login, logins)),
        ConnectionKind::Vnc(c) => (c.port, c.username.as_deref().unwrap_or("")),
        ConnectionKind::Local(_) => (0, ""),
    }
}

pub fn connection_name(host: &Host, c: &Connection, logins: &[SavedLogin]) -> String {
    let (_, user) = port_and_user(&c.kind, logins);
    match c.label.as_deref().filter(|l| !l.is_empty()) {
        Some(label) => format!("{} · {} · {label}", host.name, kind_name(&c.kind)),
        None if !user.is_empty() && !matches!(c.kind, ConnectionKind::S3(_)) => {
            format!("{} · {} · {user}", host.name, kind_name(&c.kind))
        }
        None => format!("{} · {}", host.name, kind_name(&c.kind)),
    }
}

pub fn own_secret(login: &Login) -> Option<&str> {
    match login {
        Login::Own(o) => o.secret_id.as_deref(),
        Login::Saved(_) => None,
    }
}

// A gateway that takes the RDP login, or a saved one, has no password of its own.
pub fn gateway_secret(gateway: &Option<RdpGatewayConn>) -> Option<&str> {
    gateway.as_ref().and_then(|g| g.login.as_ref()).and_then(own_secret)
}

struct KeyView {
    label: String,
    content: Option<String>,
    usable: bool,
}

fn incoming_key(k: &SshKey, payload: &PayloadV3, env: &dyn Env) -> KeyView {
    let embedded = payload.key_files.get(&k.path).and_then(|f| STANDARD.decode(f.data.as_bytes()).ok());
    match embedded {
        Some(bytes) => {
            KeyView { label: format!("{} (in export)", k.path), content: Some(digest(&bytes)), usable: true }
        }
        None => match env.read_file(&k.path) {
            Some(bytes) => KeyView { label: k.path.clone(), content: Some(digest(&bytes)), usable: true },
            None => KeyView {
                label: format!("{} (not on this machine)", k.path),
                content: None,
                usable: false,
            },
        },
    }
}

struct Collector<'a> {
    items: Vec<ReviewItem>,
    unchanged: u32,
    matches: HashMap<String, String>,
    payload: &'a PayloadV3,
    local: &'a HostsFile,
    env: &'a dyn Env,
    folder_map: HashMap<String, String>,
    tag_map: HashMap<String, String>,
    key_map: HashMap<String, String>,
    login_map: HashMap<String, String>,
    conn_map: HashMap<String, String>,
}

impl Collector<'_> {
    #[allow(clippy::too_many_arguments)]
    fn matched(
        &mut self,
        key: String,
        kind: ItemKind,
        name: String,
        local: (&str, &str),
        fields: Vec<FieldDiff>,
        (notes, notes_as_new): (Vec<String>, Vec<String>),
        parent: Option<String>,
    ) {
        self.matches.insert(key.clone(), local.0.to_string());
        if fields.is_empty() {
            if kind != ItemKind::Connection {
                self.unchanged += 1;
            }
            return;
        }
        self.items.push(ReviewItem {
            key,
            kind,
            name,
            status: ItemStatus::Conflict,
            matched: Some(LocalMatch { id: local.0.into(), name: local.1.into() }),
            fields,
            notes,
            notes_as_new,
            parent,
        });
    }

    fn added(&mut self, key: String, kind: ItemKind, name: String, notes: Vec<String>, parent: Option<String>) {
        self.items.push(ReviewItem {
            key,
            kind,
            name,
            status: ItemStatus::New,
            matched: None,
            fields: vec![],
            notes_as_new: notes.clone(),
            notes,
            parent,
        });
    }

    fn folder_name(&self, id: &str) -> String {
        self.local
            .folders
            .iter()
            .chain(&self.payload.folders)
            .find(|f| f.id == id)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| id.into())
    }

    fn connection_name_by_id(&self, id: &str) -> String {
        let find = |hosts: &[Host], logins: &[SavedLogin]| {
            hosts.iter().find_map(|h| {
                h.connections.iter().find(|c| c.id == id).map(|c| connection_name(h, c, logins))
            })
        };
        find(&self.local.hosts, &self.local.logins)
            .or_else(|| find(&self.payload.hosts, &self.payload.logins))
            .unwrap_or_else(|| id.into())
    }

    // How a login reads, from the lists it belongs to.
    fn login_label(login: &Login, logins: &[SavedLogin], keys: &[SshKey]) -> String {
        match login {
            Login::Saved(s) => match logins.iter().find(|l| l.id == s.login_id) {
                Some(l) => format!("saved login {}", l.name),
                None => "a saved login that isn't here".into(),
            },
            Login::Own(o) => {
                let key_names: Vec<&str> = o
                    .key_ids
                    .iter()
                    .filter_map(|id| keys.iter().find(|k| &k.id == id))
                    .map(|k| file_name(&k.path))
                    .collect();
                if key_names.is_empty() {
                    format!("{}, {}", o.username, auth(o.auth_method))
                } else {
                    format!("{}, {} {}", o.username, auth(o.auth_method), key_names.join(", "))
                }
            }
        }
    }

    // The same login in local terms: references to incoming logins and keys follow their matches.
    fn same_login(&self, local: &Login, incoming: &Login) -> bool {
        let map = |m: &HashMap<String, String>, id: &str| m.get(id).cloned().unwrap_or_else(|| id.to_string());
        match (local, incoming) {
            (Login::Saved(l), Login::Saved(i)) => l.login_id == map(&self.login_map, &i.login_id),
            (Login::Own(l), Login::Own(i)) => {
                l.username == i.username
                    && l.auth_method == i.auth_method
                    && l.key_ids.len() == i.key_ids.len()
                    && l.key_ids.iter().zip(&i.key_ids).all(|(a, b)| *a == map(&self.key_map, b))
            }
            _ => false,
        }
    }

    fn login_field(&self, field: &str, label: &str, local: &Login, incoming: &Login, fields: &mut Vec<FieldDiff>) {
        if !self.same_login(local, incoming) {
            fields.push(FieldDiff {
                field: field.into(),
                label: label.into(),
                local: Self::login_label(local, &self.local.logins, &self.local.keys),
                incoming: Self::login_label(incoming, &self.payload.logins, &self.payload.keys),
            });
        }
    }

    fn password_field(
        &self,
        field: &str,
        label: &str,
        local: Option<&str>,
        incoming: Option<&str>,
        fields: &mut Vec<FieldDiff>,
    ) {
        let Some(value) = incoming.and_then(|id| self.payload.secrets.get(id)) else { return };
        let current = local.and_then(|id| self.env.secret(id));
        if current.as_ref().is_some_and(|c| c.as_slice() == value.as_bytes()) {
            return;
        }
        fields.push(FieldDiff {
            field: field.into(),
            label: label.into(),
            local: if current.is_some() { "••••".into() } else { "(none)".into() },
            incoming: if current.is_some() { "•••• (different)".into() } else { "••••".into() },
        });
    }

    fn icon_field(&self, local: &IconRef, incoming: &IconRef, fields: &mut Vec<FieldDiff>, notes: &mut Vec<String>) {
        if let IconRef::Custom { path } = incoming {
            if !self.env.icon_exists(path) {
                if local != incoming {
                    notes.push("Kept your icon; the custom image isn't on this machine".into());
                }
                return;
            }
        }
        if local != incoming {
            fields.push(FieldDiff {
                field: field::ICON.into(),
                label: "Icon".into(),
                local: icon_label(local),
                incoming: icon_label(incoming),
            });
        }
    }

    // Where an incoming reference lands locally: the matched item, one arriving in the bundle,
    // an identical local id, or nowhere.
    fn reference(map: &HashMap<String, String>, id: &str, in_bundle: bool, here: bool) -> Option<String> {
        if let Some(mapped) = map.get(id) {
            return Some(mapped.clone());
        }
        (in_bundle || here).then(|| id.to_string())
    }

    fn folder_ref(&self, id: Option<&str>, notes: &mut Vec<String>) -> Option<String> {
        let id = id?;
        let in_bundle = self.payload.folders.iter().any(|f| f.id == id);
        let here = self.local.folders.iter().any(|f| f.id == id);
        let found = Self::reference(&self.folder_map, id, in_bundle, here);
        if found.is_none() {
            notes.push("Its folder isn't in the export; it will have no folder".into());
        }
        found
    }

    fn tag_names(&self, ids: &[String], tags: &[Tag]) -> String {
        let mut names: Vec<&str> = ids
            .iter()
            .filter_map(|id| tags.iter().find(|t| &t.id == id))
            .map(|t| t.name.as_str())
            .collect();
        names.sort_unstable();
        if names.is_empty() {
            "(none)".into()
        } else {
            names.join(", ")
        }
    }

    fn host_fields(&self, l: &Host, inc: &Host, notes: &mut Vec<String>) -> Vec<FieldDiff> {
        let mut fields = vec![];
        push(&mut fields, field::NAME, "Name", l.name.clone(), inc.name.clone());
        push(&mut fields, field::ADDRESS, "Address", l.address.clone(), inc.address.clone());
        let folder = self.folder_ref(inc.folder_id.as_deref(), notes);
        if l.folder_id != folder {
            let name = |f: Option<&str>| f.map(|f| self.folder_name(f)).unwrap_or_else(|| "(no folder)".into());
            fields.push(FieldDiff {
                field: field::FOLDER_ID.into(),
                label: "Folder".into(),
                local: name(l.folder_id.as_deref()),
                incoming: name(folder.as_deref()),
            });
        }
        let mut mapped: Vec<String> =
            inc.tag_ids.iter().map(|t| self.tag_map.get(t).cloned().unwrap_or_else(|| t.clone())).collect();
        mapped.sort();
        let mut current = l.tag_ids.clone();
        current.sort();
        if mapped != current {
            fields.push(FieldDiff {
                field: field::TAGS.into(),
                label: "Tags".into(),
                local: self.tag_names(&l.tag_ids, &self.local.tags),
                incoming: self.tag_names(&inc.tag_ids, &self.payload.tags),
            });
        }
        self.icon_field(&l.icon, &inc.icon, &mut fields, notes);
        push(&mut fields, field::COLOR, "Colour", text(l.color.as_deref()), text(inc.color.as_deref()));
        fields
    }

    fn jump_ref(&self, inc: Option<&str>, unchanged: bool, notes: &mut Vec<String>) -> Option<String> {
        let id = inc?;
        if unchanged {
            return Some(id.to_string());
        }
        let in_bundle = self.payload.hosts.iter().flat_map(|h| &h.connections).any(|c| c.id == id);
        let here = self.local.hosts.iter().flat_map(|h| &h.connections).any(|c| c.id == id);
        let found = Self::reference(&self.conn_map, id, in_bundle, here);
        if found.is_none() {
            notes.push("Its jump host isn't in the export; it will connect directly".into());
        }
        found
    }

    fn missing_share<'b>(&self, folder: Option<&'b SharedFolder>) -> Option<&'b str> {
        folder.map(|f| f.path.as_str()).filter(|path| !self.env.dir_exists(path))
    }

    fn gateway_label(gateway: Option<&RdpGatewayConn>, logins: &[SavedLogin], keys: &[SshKey]) -> String {
        match gateway {
            None => "None".into(),
            Some(g) => match &g.login {
                None => format!("{}:{}, same login", g.host, g.port),
                Some(l) => format!("{}:{}, {}", g.host, g.port, Self::login_label(l, logins, keys)),
            },
        }
    }

    fn connection_fields(&self, l: &Connection, inc: &Connection, notes: &mut Vec<String>) -> Vec<FieldDiff> {
        let mut fields = vec![];
        if !self.payload.legacy {
            push(&mut fields, field::LABEL, "Label", text(l.label.as_deref()), text(inc.label.as_deref()));
        }
        push(
            &mut fields,
            field::ADDRESS,
            "Different address",
            text(l.address.as_deref()),
            text(inc.address.as_deref()),
        );
        let port = |f: &mut Vec<FieldDiff>, a: u16, b: u16| push(f, field::PORT, "Port", a.to_string(), b.to_string());
        match (&l.kind, &inc.kind) {
            (ConnectionKind::Ssh(a), ConnectionKind::Ssh(b)) => {
                port(&mut fields, a.port, b.port);
                self.login_field(field::LOGIN, "Login", &a.login, &b.login, &mut fields);
                self.password_field(field::PASSWORD, "Password", own_secret(&a.login), own_secret(&b.login), &mut fields);
                // A jump host the import leaves alone is not a change, even if it points nowhere.
                let jump = self.jump_ref(b.jump_host_id.as_deref(), a.jump_host_id == b.jump_host_id, notes);
                if a.jump_host_id != jump {
                    let name = |j: Option<&str>| text(j.map(|j| self.connection_name_by_id(j)).as_deref());
                    fields.push(FieldDiff {
                        field: field::JUMP_HOST_ID.into(),
                        label: "Jump host".into(),
                        local: name(a.jump_host_id.as_deref()),
                        incoming: name(jump.as_deref()),
                    });
                }
                let same_tunnels = a.tunnels.len() == b.tunnels.len()
                    && a.tunnels.iter().zip(&b.tunnels).all(|(x, y)| same_tunnel(x, y));
                if !same_tunnels {
                    fields.push(FieldDiff {
                        field: field::TUNNELS.into(),
                        label: "Tunnels".into(),
                        local: tunnels_label(&a.tunnels),
                        incoming: tunnels_label(&b.tunnels),
                    });
                }
                push(
                    &mut fields,
                    field::APPEARANCE,
                    "Terminal look",
                    appearance_label(&a.appearance),
                    appearance_label(&b.appearance),
                );
            }
            (ConnectionKind::Sftp(a), ConnectionKind::Sftp(b)) => {
                let own_label = |c: &crate::store::hosts::SftpConn, logins: &[SavedLogin], keys: &[SshKey]| match &c.own {
                    None => "Same login as SSH".to_string(),
                    Some(o) => format!("port {}, {}", o.port, Self::login_label(&o.login, logins, keys)),
                };
                let same = match (&a.own, &b.own) {
                    (None, None) => true,
                    (Some(x), Some(y)) => x.port == y.port && self.same_login(&x.login, &y.login),
                    _ => false,
                };
                if !same {
                    fields.push(FieldDiff {
                        field: field::SFTP_LOGIN.into(),
                        label: "Login".into(),
                        local: own_label(a, &self.local.logins, &self.local.keys),
                        incoming: own_label(b, &self.payload.logins, &self.payload.keys),
                    });
                }
                self.password_field(
                    field::PASSWORD,
                    "Password",
                    a.own.as_ref().and_then(|o| own_secret(&o.login)),
                    b.own.as_ref().and_then(|o| own_secret(&o.login)),
                    &mut fields,
                );
                push(
                    &mut fields,
                    field::START_FOLDER,
                    "Start folder",
                    text(a.start_folder.as_deref()),
                    text(b.start_folder.as_deref()),
                );
            }
            (ConnectionKind::Ftp(a), ConnectionKind::Ftp(b)) => {
                port(&mut fields, a.port, b.port);
                self.login_field(field::LOGIN, "Login", &a.login, &b.login, &mut fields);
                self.password_field(field::PASSWORD, "Password", own_secret(&a.login), own_secret(&b.login), &mut fields);
                push(&mut fields, field::SECURE, "TLS (FTPS)", on_off(a.secure), on_off(b.secure));
                push(
                    &mut fields,
                    field::ALLOW_INVALID_CERT,
                    "Accept a certificate that doesn't check out",
                    on_off(a.allow_invalid_cert),
                    on_off(b.allow_invalid_cert),
                );
                push(
                    &mut fields,
                    field::IGNORE_HOSTNAME,
                    "Ignore a certificate name mismatch",
                    on_off(a.ignore_hostname),
                    on_off(b.ignore_hostname),
                );
            }
            (ConnectionKind::S3(a), ConnectionKind::S3(b)) => {
                let port_text = |p: Option<u16>| text(p.map(|p| p.to_string()).as_deref());
                push(&mut fields, field::PORT, "Port", port_text(a.port), port_text(b.port));
                push(&mut fields, field::REGION, "Region", a.region.clone(), b.region.clone());
                push(&mut fields, field::USE_TLS, "Use TLS", on_off(a.use_tls), on_off(b.use_tls));
                push(&mut fields, field::PATH_STYLE, "Path-style addressing", on_off(a.path_style), on_off(b.path_style));
                push(&mut fields, field::ACCESS_KEY_ID, "Access key ID", a.access_key_id.clone(), b.access_key_id.clone());
                push(&mut fields, field::BUCKET, "Bucket", text(a.bucket.as_deref()), text(b.bucket.as_deref()));
                self.password_field(
                    field::PASSWORD,
                    "Secret access key",
                    a.secret_id.as_deref(),
                    b.secret_id.as_deref(),
                    &mut fields,
                );
            }
            (ConnectionKind::Rdp(a), ConnectionKind::Rdp(b)) => {
                port(&mut fields, a.port, b.port);
                self.login_field(field::LOGIN, "Login", &a.login, &b.login, &mut fields);
                self.password_field(field::PASSWORD, "Password", own_secret(&a.login), own_secret(&b.login), &mut fields);
                push(&mut fields, field::DOMAIN, "Domain", text(a.domain.as_deref()), text(b.domain.as_deref()));
                push(
                    &mut fields,
                    field::KEYBOARD_LAYOUT,
                    "Keyboard layout",
                    layout_label(a.keyboard_layout),
                    layout_label(b.keyboard_layout),
                );
                push(&mut fields, field::AUDIO, "Sound", on_off(a.audio), on_off(b.audio));
                match self.missing_share(b.shared_folder.as_ref()) {
                    None => push(
                        &mut fields,
                        field::SHARED_FOLDER,
                        "Shared folder",
                        folder_label(a.shared_folder.as_ref()),
                        folder_label(b.shared_folder.as_ref()),
                    ),
                    Some(path) if a.shared_folder.is_none() => {
                        notes.push(format!("Shared folder left out; not on this machine: {path}"));
                    }
                    Some(path) if a.shared_folder != b.shared_folder => {
                        notes.push(format!("Kept your local shared folder; not on this machine: {path}"));
                    }
                    Some(_) => {}
                }
                let same_gateway = match (&a.gateway, &b.gateway) {
                    (None, None) => true,
                    (Some(x), Some(y)) => {
                        x.host == y.host
                            && x.port == y.port
                            && match (&x.login, &y.login) {
                                (None, None) => true,
                                (Some(xl), Some(yl)) => self.same_login(xl, yl),
                                _ => false,
                            }
                    }
                    _ => false,
                };
                if !same_gateway {
                    fields.push(FieldDiff {
                        field: field::GATEWAY.into(),
                        label: "RD Gateway".into(),
                        local: Self::gateway_label(a.gateway.as_ref(), &self.local.logins, &self.local.keys),
                        incoming: Self::gateway_label(b.gateway.as_ref(), &self.payload.logins, &self.payload.keys),
                    });
                }
                self.password_field(
                    field::GATEWAY_PASSWORD,
                    "Gateway password",
                    gateway_secret(&a.gateway),
                    gateway_secret(&b.gateway),
                    &mut fields,
                );
            }
            (ConnectionKind::Vnc(a), ConnectionKind::Vnc(b)) => {
                port(&mut fields, a.port, b.port);
                push(&mut fields, field::USERNAME, "User", text(a.username.as_deref()), text(b.username.as_deref()));
                self.password_field(field::PASSWORD, "Password", a.secret_id.as_deref(), b.secret_id.as_deref(), &mut fields);
            }
            _ => {}
        }
        fields
    }

    fn new_connection_notes(&self, inc: &Connection) -> Vec<String> {
        let mut notes = vec![];
        match &inc.kind {
            ConnectionKind::Ssh(s) => {
                self.jump_ref(s.jump_host_id.as_deref(), false, &mut notes);
            }
            ConnectionKind::Rdp(r) => {
                if let Some(path) = self.missing_share(r.shared_folder.as_ref()) {
                    notes.push(format!("Shared folder left out; not on this machine: {path}"));
                }
            }
            _ => {}
        }
        notes
    }
}

pub fn plan(local: &HostsFile, payload: &PayloadV3, env: &dyn Env) -> Plan {
    let mut c = Collector {
        items: vec![],
        unchanged: 0,
        matches: HashMap::new(),
        payload,
        local,
        env,
        folder_map: HashMap::new(),
        tag_map: HashMap::new(),
        key_map: HashMap::new(),
        login_map: HashMap::new(),
        conn_map: HashMap::new(),
    };

    let mut folders: Vec<&Folder> = payload.folders.iter().collect();
    folders.sort_by_key(|f| folder_depth(f, &payload.folders));
    let (found, folder_map) = match_items(
        &local.folders,
        &folders,
        |l| (l.id.as_str(), l.name.as_str()),
        |f| (f.id.as_str(), f.name.as_str()),
        |_, _| true,
        |l, f, map| {
            let parent = f.parent_id.as_ref().map(|p| map.get(p).unwrap_or(p));
            l.name == f.name && l.parent_id.as_ref() == parent
        },
    );
    c.folder_map = folder_map;
    for (f, found) in folders.into_iter().zip(found) {
        let key = ItemKind::Folder.key(&f.id);
        match found {
            Some(l) => {
                let mut notes = vec![];
                let mut fields = vec![];
                push(&mut fields, field::NAME, "Name", l.name.clone(), f.name.clone());
                let parent = f.parent_id.as_ref().map(|p| c.folder_map.get(p).cloned().unwrap_or_else(|| p.clone()));
                if l.parent_id != parent {
                    let name = |p: Option<&str>| p.map(|p| c.folder_name(p)).unwrap_or_else(|| "(top level)".into());
                    fields.push(FieldDiff {
                        field: field::PARENT_ID.into(),
                        label: "Parent folder".into(),
                        local: name(l.parent_id.as_deref()),
                        incoming: name(parent.as_deref()),
                    });
                }
                c.icon_field(&l.icon, &f.icon, &mut fields, &mut notes);
                c.matched(key, ItemKind::Folder, f.name.clone(), (&l.id, &l.name), fields, (notes, vec![]), None);
            }
            None => c.added(key, ItemKind::Folder, f.name.clone(), vec![], None),
        }
    }

    let (found, tag_map) = match_items(
        &local.tags,
        &payload.tags,
        |l| (l.id.as_str(), l.name.as_str()),
        |t| (t.id.as_str(), t.name.as_str()),
        |_, _| true,
        |l, t, _| l.name == t.name,
    );
    c.tag_map = tag_map;
    for (t, found) in payload.tags.iter().zip(found) {
        let key = ItemKind::Tag.key(&t.id);
        match found {
            Some(l) => {
                let mut fields = vec![];
                push(&mut fields, field::NAME, "Name", l.name.clone(), t.name.clone());
                push(&mut fields, field::COLOR, "Colour", l.color.clone(), t.color.clone());
                push(&mut fields, field::SHOW_COLOR, "Show the colour", on_off(l.show_color), on_off(t.show_color));
                push(
                    &mut fields,
                    field::ASK_BEFORE_CLOSE,
                    "Ask before closing",
                    on_off(l.ask_before_close),
                    on_off(t.ask_before_close),
                );
                push(
                    &mut fields,
                    field::ASK_BEFORE_PASTE,
                    "Ask before pasting several lines",
                    on_off(l.ask_before_paste),
                    on_off(t.ask_before_paste),
                );
                push(
                    &mut fields,
                    field::TERMINAL_COLORS,
                    "Terminal colours",
                    text(l.terminal_colors.as_deref()),
                    text(t.terminal_colors.as_deref()),
                );
                c.matched(key, ItemKind::Tag, t.name.clone(), (&l.id, &l.name), fields, (vec![], vec![]), None);
            }
            None => c.added(key, ItemKind::Tag, t.name.clone(), vec![], None),
        }
    }

    let (found, key_map) = match_items(
        &local.keys,
        &payload.keys,
        |l| (l.id.as_str(), l.path.as_str()),
        |k| (k.id.as_str(), k.path.as_str()),
        |_, _| true,
        |l, k, _| l.path == k.path,
    );
    c.key_map = key_map;
    for (k, found) in payload.keys.iter().zip(found) {
        let key = ItemKind::Key.key(&k.id);
        let view = incoming_key(k, payload, env);
        let name = file_name(&k.path).to_string();
        let missing = (!view.usable).then(|| format!("Key file not on this machine: {}", k.path));
        match found {
            Some(l) => {
                let mut fields = vec![];
                let mut notes = vec![];
                if view.usable {
                    let here = env.read_file(&l.path).map(|b| digest(&b));
                    let same = match (&here, &view.content) {
                        (Some(a), Some(b)) => a == b,
                        _ => l.path == k.path,
                    };
                    if !same {
                        fields.push(FieldDiff {
                            field: field::KEY_FILE.into(),
                            label: "Key file".into(),
                            local: l.path.clone(),
                            incoming: view.label.clone(),
                        });
                    }
                } else if l.path != k.path {
                    notes.push(format!("Kept your local key file; not on this machine: {}", k.path));
                }
                c.password_field(field::PASSPHRASE, "Passphrase", l.secret_id.as_deref(), k.secret_id.as_deref(), &mut fields);
                let as_new = missing.into_iter().collect();
                c.matched(key, ItemKind::Key, name, (&l.id, file_name(&l.path)), fields, (notes, as_new), None);
            }
            None => c.added(key, ItemKind::Key, name, missing.into_iter().collect(), None),
        }
    }

    let (found, login_map) = match_items(
        &local.logins,
        &payload.logins,
        |l| (l.id.as_str(), l.name.as_str()),
        |l| (l.id.as_str(), l.name.as_str()),
        |_, _| true,
        |l, i, _| l.name == i.name && l.username == i.username,
    );
    c.login_map = login_map;
    for (inc, found) in payload.logins.iter().zip(found) {
        let key = ItemKind::Login.key(&inc.id);
        match found {
            Some(l) => {
                let mut fields = vec![];
                push(&mut fields, field::NAME, "Name", l.name.clone(), inc.name.clone());
                push(&mut fields, field::USERNAME, "User", l.username.clone(), inc.username.clone());
                push(&mut fields, field::AUTH_METHOD, "Sign in with", auth(l.auth_method), auth(inc.auth_method));
                let mapped: Vec<String> =
                    inc.key_ids.iter().map(|k| c.key_map.get(k).cloned().unwrap_or_else(|| k.clone())).collect();
                if mapped != l.key_ids {
                    let names = |ids: &[String], keys: &[SshKey]| {
                        let n: Vec<&str> = ids
                            .iter()
                            .filter_map(|id| keys.iter().find(|k| &k.id == id))
                            .map(|k| file_name(&k.path))
                            .collect();
                        if n.is_empty() { "(none)".to_string() } else { n.join(", ") }
                    };
                    fields.push(FieldDiff {
                        field: field::KEYS.into(),
                        label: "Keys".into(),
                        local: names(&l.key_ids, &local.keys),
                        incoming: names(&inc.key_ids, &payload.keys),
                    });
                }
                c.password_field(field::PASSWORD, "Password", l.secret_id.as_deref(), inc.secret_id.as_deref(), &mut fields);
                c.matched(key, ItemKind::Login, inc.name.clone(), (&l.id, &l.name), fields, (vec![], vec![]), None);
            }
            None => c.added(key, ItemKind::Login, inc.name.clone(), vec![], None),
        }
    }

    let local_hosts: Vec<&Host> = local.hosts.iter().filter(|h| h.id != THIS_COMPUTER).collect();
    let incoming_hosts: Vec<&Host> = payload.hosts.iter().filter(|h| h.id != THIS_COMPUTER).collect();
    let (found, _) = match_items(
        &local_hosts,
        &incoming_hosts,
        |l| (l.id.as_str(), l.name.as_str()),
        |h| (h.id.as_str(), h.name.as_str()),
        |_, _| true,
        |l, h, _| address_key(&l.address) == address_key(&h.address),
    );
    let pairs: Vec<(&Host, Option<&Host>)> = incoming_hosts.iter().copied().zip(found.into_iter().map(|f| f.copied())).collect();

    // Connections first, so a jump host anywhere in the bundle maps to the connection it matched.
    let mut conn_found: HashMap<&str, Vec<Option<&Connection>>> = HashMap::new();
    for (inc, found) in &pairs {
        if let Some(l) = found {
            let (matched, map) = match_items(
                &l.connections,
                &inc.connections,
                |c| (c.id.as_str(), c.label.as_deref().unwrap_or("")),
                |c| (c.id.as_str(), c.label.as_deref().unwrap_or("")),
                |a, b| same_type(&a.kind, &b.kind),
                |a, b, _| {
                    same_type(&a.kind, &b.kind)
                        && port_and_user(&a.kind, &local.logins) == port_and_user(&b.kind, &payload.logins)
                },
            );
            c.conn_map.extend(map);
            conn_found.insert(&inc.id, matched);
        }
    }

    for (inc, found) in pairs {
        let key = ItemKind::Host.key(&inc.id);
        let mut as_new = vec![];
        c.folder_ref(inc.folder_id.as_deref(), &mut as_new);
        if matches!(&inc.icon, IconRef::Custom { path } if !env.icon_exists(path)) {
            as_new.push("The custom icon isn't on this machine; using the default icon".into());
        }
        match found {
            Some(l) => {
                let mut notes = vec![];
                let fields = if payload.legacy { vec![] } else { c.host_fields(l, inc, &mut notes) };
                let matched = conn_found.remove(inc.id.as_str()).unwrap_or_default();
                let before = c.items.len();
                let mut conn_items = vec![];
                for (ic, lc) in inc.connections.iter().zip(matched) {
                    let ckey = ItemKind::Connection.key(&ic.id);
                    let name = connection_name(inc, ic, &payload.logins);
                    match lc {
                        Some(lc) => {
                            let mut cnotes = vec![];
                            let cfields = c.connection_fields(lc, ic, &mut cnotes);
                            let cas_new = c.new_connection_notes(ic);
                            let lname = connection_name(l, lc, &local.logins);
                            c.matched(
                                ckey,
                                ItemKind::Connection,
                                name,
                                (&lc.id, &lname),
                                cfields,
                                (cnotes, cas_new),
                                Some(key.clone()),
                            );
                        }
                        None => {
                            let notes = c.new_connection_notes(ic);
                            c.added(ckey, ItemKind::Connection, name, notes, Some(key.clone()));
                        }
                    }
                }
                conn_items.extend(c.items.drain(before..));
                c.matches.insert(key.clone(), l.id.clone());
                if !fields.is_empty() {
                    c.items.push(ReviewItem {
                        key,
                        kind: ItemKind::Host,
                        name: inc.name.clone(),
                        status: ItemStatus::Conflict,
                        matched: Some(LocalMatch { id: l.id.clone(), name: l.name.clone() }),
                        fields,
                        notes,
                        notes_as_new: as_new,
                        parent: None,
                    });
                } else if conn_items.is_empty() {
                    c.unchanged += 1;
                }
                // Changed connections on an otherwise unchanged host come without a host item.
                c.items.extend(conn_items);
            }
            None => {
                c.added(key.clone(), ItemKind::Host, inc.name.clone(), as_new, None);
                for ic in &inc.connections {
                    let notes = c.new_connection_notes(ic);
                    let name = connection_name(inc, ic, &payload.logins);
                    c.added(ItemKind::Connection.key(&ic.id), ItemKind::Connection, name, notes, Some(key.clone()));
                }
            }
        }
    }

    Plan { items: c.items, unchanged: c.unchanged, matches: c.matches }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::hosts::fixtures::{conn, file, folder, host, own, ssh, vnc};
    use crate::store::hosts::{OwnLogin, RdpConn, SshConn, VncConn};
    use crate::store::model::{Group, Profile, ProfileKey};
    use crate::transfer::bundle::KeyFile;
    use crate::transfer::MapEnv;
    use zeroize::Zeroizing;

    fn rows(item: &ReviewItem) -> Vec<(&str, &str, &str)> {
        item.fields.iter().map(|f| (f.field.as_str(), f.local.as_str(), f.incoming.as_str())).collect()
    }

    fn payload(hosts: Vec<Host>) -> PayloadV3 {
        PayloadV3 { hosts, ..Default::default() }
    }

    fn item<'a>(plan: &'a Plan, key: &str) -> &'a ReviewItem {
        plan.items.iter().find(|i| i.key == key).unwrap_or_else(|| panic!("no item {key}"))
    }

    fn profile(id: &str, name: &str, host: &str, port: u16) -> Profile {
        Profile {
            id: id.into(),
            name: name.into(),
            group_id: Some("g1".into()),
            host: host.into(),
            port,
            username: "me".into(),
            auth_method: AuthMethod::Key,
            key_path: None,
            keys: vec![ProfileKey { path: "/k/id".into(), secret_id: None }],
            secret_id: None,
            icon: IconRef::default(),
            order: 0,
            jump_host_id: None,
            tunnels: vec![],
            appearance: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn a_v2_bundle_of_this_machine_is_unchanged() {
        let mut jumped = profile("p2", "web", "10.0.0.2", 22);
        jumped.jump_host_id = Some("p1".into());
        let old = ProfileStore {
            version: 1,
            groups: vec![Group {
                id: "g1".into(),
                name: "Work".into(),
                parent_id: None,
                icon: IconRef::default(),
                order: 0,
                extra: Default::default(),
            }],
            profiles: vec![profile("p1", "bastion", "bastion.corp", 22), jumped],
            vnc_profiles: vec![crate::store::model::VncProfile {
                id: "v1".into(),
                name: "web screen".into(),
                host: "10.0.0.2".into(),
                port: 5900,
                username: None,
                secret_id: None,
                icon: IconRef::default(),
                order: 0,
                extra: Default::default(),
            }],
            ..Default::default()
        };
        let (local, _) = migrate(&old, &[]);
        let bundle = LegacyPayload {
            groups: old.groups.clone(),
            profiles: old.profiles.clone(),
            vnc_profiles: old.vnc_profiles.clone(),
            ..Default::default()
        };
        let mut env = MapEnv::default();
        env.files.insert("/k/id".into(), b"KEY".to_vec());
        let planned = plan(&local, &from_legacy(bundle), &env);
        assert!(planned.items.is_empty(), "{:?}", planned.items);
        let expected = local.hosts.len() + local.folders.len() + local.keys.len();
        assert_eq!(planned.unchanged as usize, expected);
    }

    // 0.5.0 exported per kind, so a v2 bundle can hold only some of a host's profiles: the host's
    // name, folder and the connection labels then come out different without anything having changed.
    #[test]
    fn a_partial_v2_bundle_of_this_machine_is_unchanged() {
        let old = ProfileStore {
            version: 1,
            groups: vec![Group {
                id: "g1".into(),
                name: "Work".into(),
                parent_id: None,
                icon: IconRef::default(),
                order: 0,
                extra: Default::default(),
            }],
            profiles: vec![profile("p1", "nas", "nas.local", 22)],
            vnc_profiles: vec![crate::store::model::VncProfile {
                id: "v1".into(),
                name: "nas desk".into(),
                host: "nas.local".into(),
                port: 5900,
                username: None,
                secret_id: None,
                icon: IconRef::default(),
                order: 0,
                extra: Default::default(),
            }],
            ..Default::default()
        };
        let (local, _) = migrate(&old, &[]);
        let bundle = LegacyPayload { vnc_profiles: old.vnc_profiles.clone(), ..Default::default() };
        let planned = plan(&local, &from_legacy(bundle), &MapEnv::default());
        assert!(planned.items.is_empty(), "{:?}", planned.items);
    }

    #[test]
    fn hosts_match_by_address_and_name_then_address() {
        let local = file(vec![
            host("l1", "nas", "nas.local", vec![vnc("lc1", None)]),
            host("l2", "other", "nas.local", vec![vnc("lc2", None)]),
            host("l3", "box", "10.0.0.9", vec![vnc("lc3", None)]),
        ]);
        let incoming = payload(vec![
            host("i1", "nas", "NAS.local.", vec![vnc("ic1", None)]),
            host("i2", "renamed", "10.0.0.9", vec![vnc("ic2", None)]),
        ]);
        let planned = plan(&local, &incoming, &MapEnv::default());
        assert_eq!(planned.matches.get("host:i1").map(String::as_str), Some("l1"));
        assert_eq!(planned.matches.get("host:i2").map(String::as_str), Some("l3"));
        assert_eq!(rows(item(&planned, "host:i2")), [("name", "box", "renamed")]);
    }

    #[test]
    fn an_ambiguous_address_without_a_name_match_is_new() {
        let local = file(vec![
            host("l1", "a", "nas.local", vec![]),
            host("l2", "b", "nas.local", vec![]),
        ]);
        let planned = plan(&local, &payload(vec![host("i1", "c", "nas.local", vec![])]), &MapEnv::default());
        assert_eq!(item(&planned, "host:i1").status, ItemStatus::New);
    }

    #[test]
    fn connections_match_by_type_port_and_user_inside_a_host() {
        let local = file(vec![host(
            "l",
            "nas",
            "nas.local",
            vec![ssh("ls-admin", 22, own("admin", None)), ssh("ls-root", 22, own("root", None))],
        )]);
        let incoming = payload(vec![host(
            "i",
            "nas",
            "nas.local",
            vec![ssh("is-root", 22, own("root", None)), ssh("is-deploy", 22, own("deploy", None))],
        )]);
        let planned = plan(&local, &incoming, &MapEnv::default());
        assert_eq!(planned.matches.get("conn:is-root").map(String::as_str), Some("ls-root"));
        let new = item(&planned, "conn:is-deploy");
        assert_eq!(new.status, ItemStatus::New);
        assert_eq!(new.parent.as_deref(), Some("host:i"));
        assert_eq!(new.name, "nas · SSH · deploy");
        assert!(planned.items.iter().all(|i| i.key != "host:i"));
    }

    #[test]
    fn an_id_match_never_crosses_connection_types() {
        let local = file(vec![host("h", "nas", "nas.local", vec![vnc("same", None)])]);
        let incoming = payload(vec![host("h", "nas", "nas.local", vec![ssh("same", 22, own("u", None))])]);
        let planned = plan(&local, &incoming, &MapEnv::default());
        assert_eq!(item(&planned, "conn:same").status, ItemStatus::New);
    }

    #[test]
    fn a_new_hosts_connections_are_new_children() {
        let incoming = payload(vec![host(
            "i",
            "nas",
            "nas.local",
            vec![ssh("c1", 22, own("admin", None)), vnc("c2", None)],
        )]);
        let planned = plan(&file(vec![]), &incoming, &MapEnv::default());
        let keys: Vec<_> = planned.items.iter().map(|i| (i.key.as_str(), i.parent.as_deref())).collect();
        assert_eq!(keys, [("host:i", None), ("conn:c1", Some("host:i")), ("conn:c2", Some("host:i"))]);
    }

    #[test]
    fn incoming_this_computer_is_skipped() {
        let incoming = payload(vec![host(THIS_COMPUTER, "This computer", "", vec![])]);
        let planned = plan(&file(vec![]), &incoming, &MapEnv::default());
        assert!(planned.items.is_empty());
        assert_eq!(planned.unchanged, 0);
    }

    #[test]
    fn a_changed_connection_lists_its_fields() {
        let mut lc = vnc("c", Some("old"));
        lc.label = Some("desk".into());
        let local = file(vec![host("h", "nas", "nas.local", vec![lc])]);
        let mut ic = vnc("c", Some("new"));
        if let ConnectionKind::Vnc(VncConn { port, username, .. }) = &mut ic.kind {
            *port = 5901;
            *username = Some("faye".into());
        }
        let mut incoming = payload(vec![host("h", "nas", "nas.local", vec![ic])]);
        incoming.secrets.insert("new".into(), Zeroizing::new("pw".into()));
        let mut env = MapEnv::default();
        env.secrets.insert("old".into(), b"other".to_vec());
        let planned = plan(&local, &incoming, &env);
        assert_eq!(
            rows(item(&planned, "conn:c")),
            [
                ("label", "desk", "(none)"),
                ("port", "5900", "5901"),
                ("username", "(none)", "faye"),
                ("password", "••••", "•••• (different)"),
            ]
        );
    }

    #[test]
    fn password_row_only_when_the_bundle_carries_a_different_one() {
        let local = file(vec![host("h", "nas", "a", vec![ssh("c", 22, own("u", Some("old")))])]);
        let incoming = payload(vec![host("h", "nas", "a", vec![ssh("c", 22, own("u", Some("x")))])]);
        let mut env = MapEnv::default();
        env.secrets.insert("old".into(), b"pw".to_vec());
        assert!(plan(&local, &incoming, &env).items.is_empty());

        let mut same = incoming.clone();
        same.secrets.insert("x".into(), Zeroizing::new("pw".into()));
        assert!(plan(&local, &same, &env).items.is_empty());

        let mut other = incoming;
        other.secrets.insert("x".into(), Zeroizing::new("new".into()));
        assert_eq!(rows(item(&plan(&local, &other, &env), "conn:c")), [("password", "••••", "•••• (different)")]);
    }

    #[test]
    fn a_missing_key_file_keeps_the_local_one_with_a_note() {
        let mut local = file(vec![]);
        local.keys.push(SshKey { id: "k".into(), path: "C:\\keys\\id".into(), secret_id: None, extra: Default::default() });
        let mut incoming = payload(vec![]);
        incoming.keys.push(SshKey { id: "k".into(), path: "/Users/me/.ssh/id".into(), secret_id: None, extra: Default::default() });
        let mut env = MapEnv::default();
        env.files.insert("C:\\keys\\id".into(), b"KEY".to_vec());
        let planned = plan(&local, &incoming, &env);
        assert!(planned.items.is_empty());
        assert_eq!(planned.unchanged, 1);

        let mut with_note = incoming.clone();
        with_note.keys[0].secret_id = Some("pp".into());
        with_note.secrets.insert("pp".into(), Zeroizing::new("pass".into()));
        let planned = plan(&local, &with_note, &env);
        let key = item(&planned, "key:k");
        assert_eq!(rows(key), [("passphrase", "(none)", "••••")]);
        assert_eq!(key.notes, ["Kept your local key file; not on this machine: /Users/me/.ssh/id"]);
        assert_eq!(key.notes_as_new, ["Key file not on this machine: /Users/me/.ssh/id"]);
    }

    #[test]
    fn an_embedded_key_with_other_content_is_a_key_file_row() {
        let mut local = file(vec![]);
        local.keys.push(SshKey { id: "k".into(), path: "/k/id".into(), secret_id: None, extra: Default::default() });
        let mut incoming = payload(vec![]);
        incoming.keys = local.keys.clone();
        incoming.key_files.insert(
            "/k/id".into(),
            KeyFile { file_name: "id".into(), data: Zeroizing::new(STANDARD.encode(b"NEW")) },
        );
        let mut env = MapEnv::default();
        env.files.insert("/k/id".into(), b"OLD".to_vec());
        assert_eq!(rows(item(&plan(&local, &incoming, &env), "key:k")), [("keyFile", "/k/id", "/k/id (in export)")]);
    }

    #[test]
    fn a_login_compares_keys_through_their_matches() {
        let key_login = |key: &str| {
            Login::Own(OwnLogin { username: "u".into(), auth_method: AuthMethod::Key, key_ids: vec![key.into()], secret_id: None, extra: Default::default() })
        };
        let mut local = file(vec![host("h", "nas", "a", vec![ssh("c", 22, key_login("lk"))])]);
        local.keys.push(SshKey { id: "lk".into(), path: "/k/id".into(), secret_id: None, extra: Default::default() });
        let mut incoming = payload(vec![host("h", "nas", "a", vec![ssh("c", 22, key_login("ik"))])]);
        incoming.keys.push(SshKey { id: "ik".into(), path: "/k/id".into(), secret_id: None, extra: Default::default() });
        let mut env = MapEnv::default();
        env.files.insert("/k/id".into(), b"KEY".to_vec());
        let planned = plan(&local, &incoming, &env);
        assert!(planned.items.is_empty(), "{:?}", planned.items);
    }

    #[test]
    fn folders_match_by_name_and_references_are_remapped() {
        let mut lh = host("h", "nas", "a", vec![]);
        lh.folder_id = Some("lf".into());
        let mut local = file(vec![lh]);
        local.folders = vec![folder("lf", None)];
        local.folders[0].name = "Homelab".into();
        let mut ih = host("h", "nas", "a", vec![]);
        ih.folder_id = Some("if".into());
        let mut incoming = payload(vec![ih]);
        incoming.folders = vec![folder("if", None)];
        incoming.folders[0].name = "Homelab".into();
        let planned = plan(&local, &incoming, &MapEnv::default());
        assert!(planned.items.is_empty(), "{:?}", planned.items);
        assert_eq!(planned.matches.get("folder:if").map(String::as_str), Some("lf"));
    }

    #[test]
    fn a_folder_not_in_the_export_is_noted() {
        let mut ih = host("h", "nas", "a", vec![]);
        ih.folder_id = Some("gone".into());
        let planned = plan(&file(vec![]), &payload(vec![ih]), &MapEnv::default());
        assert_eq!(item(&planned, "host:h").notes, ["Its folder isn't in the export; it will have no folder"]);
    }

    #[test]
    fn a_jump_host_maps_to_the_connection_it_matched() {
        let jumping = |id: &str, jump: &str| {
            let mut c = ssh(id, 22, own("u", None));
            if let ConnectionKind::Ssh(SshConn { jump_host_id, .. }) = &mut c.kind {
                *jump_host_id = Some(jump.into());
            }
            c
        };
        let local = file(vec![
            host("lb", "bastion", "bastion.corp", vec![ssh("l-bastion", 22, own("u", None))]),
            host("lt", "target", "10.0.0.9", vec![jumping("l-target", "l-bastion")]),
        ]);
        let incoming = payload(vec![
            host("ib", "bastion", "bastion.corp", vec![ssh("i-bastion", 22, own("u", None))]),
            host("it", "target", "10.0.0.9", vec![jumping("i-target", "i-bastion")]),
        ]);
        let planned = plan(&local, &incoming, &MapEnv::default());
        assert!(planned.items.is_empty(), "{:?}", planned.items);
    }

    #[test]
    fn rdp_shared_folder_not_on_this_machine_is_never_offered() {
        let rdp = |id: &str, share: Option<&str>| {
            conn(
                id,
                ConnectionKind::Rdp(RdpConn {
                    port: 3389,
                    login: own("u", None),
                    domain: None,
                    keyboard_layout: None,
                    audio: true,
                    shared_folder: share.map(|p| SharedFolder { path: p.into(), name: "s".into(), extra: Default::default() }),
                    gateway: None,
                    extra: Default::default(),
                }),
            )
        };
        let local = file(vec![host("h", "office", "a", vec![rdp("c", None)])]);
        let incoming = payload(vec![host("h", "office", "a", vec![rdp("c", Some("/Users/me/share"))])]);
        let planned = plan(&local, &incoming, &MapEnv::default());
        assert!(planned.items.is_empty());

        let new = plan(&file(vec![]), &incoming, &MapEnv::default());
        assert_eq!(item(&new, "conn:c").notes, ["Shared folder left out; not on this machine: /Users/me/share"]);
    }

    #[test]
    fn rdp_layout_names_are_the_ones_the_profile_dialog_offers() {
        let dialog = include_str!("../../../src/lib/rdpLayouts.ts");
        let table = dialog.split("const LAYOUTS").nth(1).unwrap().split("\n]\n").next().unwrap();
        let offered: Vec<(u32, &str)> = table
            .lines()
            .filter_map(|line| {
                let entry = line.trim().strip_prefix("[0x")?.strip_suffix("'],")?;
                let (id, name) = entry.split_once(", '")?;
                Some((u32::from_str_radix(id, 16).unwrap(), name))
            })
            .collect();
        assert_eq!(offered.len(), 24);
        assert_eq!(offered, RDP_LAYOUTS);
    }
}
