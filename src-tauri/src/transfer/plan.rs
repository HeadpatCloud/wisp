use std::collections::{HashMap, HashSet};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use sha2::{Digest, Sha256};

use super::bundle::Payload;
use super::{Env, FieldDiff, ItemKind, ItemStatus, LocalMatch, ReviewItem};
use crate::store::model::{
    AuthMethod, Group, IconRef, ProfileAppearance, ProfileKey, ProfileStore, Tunnel, TunnelKind,
};
use crate::store::normalize_keys;

pub struct Plan {
    pub items: Vec<ReviewItem>,
    pub unchanged: u32,
    pub matches: HashMap<String, String>,
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn group_depth(g: &Group, all: &[Group]) -> usize {
    let mut depth = 0;
    let mut parent = g.parent_id.as_deref();
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

fn find_match<'a, T>(
    locals: &'a [T],
    claimed: &mut HashSet<String>,
    id_of: impl Fn(&T) -> &str,
    name_of: impl Fn(&T) -> &str,
    same_server: impl Fn(&T) -> bool,
    incoming_id: &str,
    incoming_name: &str,
) -> Option<&'a T> {
    let free = |l: &&T| !claimed.contains(id_of(l));
    let found = locals.iter().filter(free).find(|l| id_of(l) == incoming_id).or_else(|| {
        let candidates: Vec<&T> = locals.iter().filter(free).filter(|l| same_server(l)).collect();
        match candidates.len() {
            0 => None,
            1 => Some(candidates[0]),
            _ => {
                let named: Vec<&T> =
                    candidates.into_iter().filter(|l| name_of(l) == incoming_name).collect();
                if named.len() == 1 { Some(named[0]) } else { None }
            }
        }
    });
    if let Some(l) = found {
        claimed.insert(id_of(l).to_string());
    }
    found
}

fn push(fields: &mut Vec<FieldDiff>, field: &str, label: &str, local: String, incoming: String) {
    if local != incoming {
        fields.push(FieldDiff { field: field.into(), label: label.into(), local, incoming });
    }
}

fn text(v: Option<&str>) -> String {
    v.filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| "(none)".into())
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

fn appearance_label(a: &Option<ProfileAppearance>) -> String {
    match a {
        None => "(default)".into(),
        Some(a) => format!(
            "theme {}, font {} {}",
            a.theme.as_deref().unwrap_or("default"),
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
            match (&t.target_host, t.target_port) {
                (Some(h), Some(p)) => format!("{kind} {}:{} -> {h}:{p}", t.bind_host, t.bind_port),
                _ => format!("{kind} {}:{}", t.bind_host, t.bind_port),
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn icon_field(
    local: &IconRef,
    incoming: &IconRef,
    env: &dyn Env,
    fields: &mut Vec<FieldDiff>,
    notes: &mut Vec<String>,
) {
    if let IconRef::Custom { path } = incoming {
        if !env.icon_exists(path) {
            if local != incoming {
                notes.push("Kept your icon; the custom image isn't on this machine".into());
            }
            return;
        }
    }
    if local != incoming {
        fields.push(FieldDiff {
            field: "icon".into(),
            label: "Icon".into(),
            local: icon_label(local),
            incoming: icon_label(incoming),
        });
    }
}

struct KeyView {
    label: String,
    content: Option<String>,
    passphrase: Option<String>,
    usable: bool,
}

fn incoming_keys(keys: &[ProfileKey], payload: &Payload, env: &dyn Env) -> Vec<KeyView> {
    keys.iter()
        .map(|k| {
            let embedded = payload
                .key_files
                .get(&k.path)
                .and_then(|f| STANDARD.decode(f.data.as_bytes()).ok());
            let (content, usable, label) = match embedded {
                Some(bytes) => (Some(digest(&bytes)), true, format!("{} (in export)", k.path)),
                None => match env.read_file(&k.path) {
                    Some(bytes) => (Some(digest(&bytes)), true, k.path.clone()),
                    None => (None, false, format!("{} (not on this machine)", k.path)),
                },
            };
            let passphrase = k
                .secret_id
                .as_ref()
                .and_then(|id| payload.secrets.get(id))
                .map(|v| digest(v.as_bytes()));
            KeyView { label, content, passphrase, usable }
        })
        .collect()
}

fn local_keys(keys: &[ProfileKey], env: &dyn Env) -> Vec<KeyView> {
    keys.iter()
        .map(|k| KeyView {
            label: k.path.clone(),
            content: env.read_file(&k.path).map(|b| digest(&b)),
            passphrase: k.secret_id.as_ref().and_then(|id| env.secret(id)).map(|v| digest(&v)),
            usable: true,
        })
        .collect()
}

fn keys_label(keys: &[KeyView]) -> String {
    if keys.is_empty() {
        return "(none)".into();
    }
    keys.iter().map(|k| k.label.clone()).collect::<Vec<_>>().join(", ")
}

fn keys_field(
    local: Option<&[ProfileKey]>,
    incoming: &[ProfileKey],
    payload: &Payload,
    env: &dyn Env,
    fields: &mut Vec<FieldDiff>,
    notes: &mut Vec<String>,
) {
    let inc = incoming_keys(incoming, payload, env);
    let missing: Vec<&str> = incoming
        .iter()
        .zip(&inc)
        .filter(|(_, v)| !v.usable)
        .map(|(k, _)| k.path.as_str())
        .collect();
    match local {
        Some(local) if !local.is_empty() && !missing.is_empty() => {
            notes.push(format!(
                "Kept your local keys; not on this machine: {}",
                missing.join(", ")
            ));
        }
        Some(local) => {
            let loc = local_keys(local, env);
            let same = loc.len() == inc.len()
                && loc.iter().zip(&inc).all(|(l, i)| {
                    let content = match (&l.content, &i.content) {
                        (Some(a), Some(b)) => a == b,
                        _ => l.label == i.label,
                    };
                    content && (i.passphrase.is_none() || i.passphrase == l.passphrase)
                });
            if !same {
                fields.push(FieldDiff {
                    field: "keys".into(),
                    label: "Private keys".into(),
                    local: keys_label(&loc),
                    incoming: keys_label(&inc),
                });
            }
            if !missing.is_empty() {
                notes.push(format!("Key file not on this machine: {}", missing.join(", ")));
            }
        }
        None => {
            if !missing.is_empty() {
                notes.push(format!("Key file not on this machine: {}", missing.join(", ")));
            }
        }
    }
}

fn password_field(
    label: &str,
    local: Option<&str>,
    incoming: Option<&str>,
    payload: &Payload,
    env: &dyn Env,
    fields: &mut Vec<FieldDiff>,
) {
    let Some(value) = incoming.and_then(|id| payload.secrets.get(id)) else { return };
    let current = local.and_then(|id| env.secret(id));
    if current.as_ref().is_some_and(|c| c.as_slice() == value.as_bytes()) {
        return;
    }
    fields.push(FieldDiff {
        field: "password".into(),
        label: label.into(),
        local: if current.is_some() { "••••".into() } else { "(none)".into() },
        incoming: "•••• (different)".into(),
    });
}

struct Collector<'a> {
    items: Vec<ReviewItem>,
    unchanged: u32,
    matches: HashMap<String, String>,
    payload: &'a Payload,
    local: &'a ProfileStore,
}

impl Collector<'_> {
    fn matched(
        &mut self,
        key: String,
        kind: ItemKind,
        name: &str,
        local: (&str, &str),
        fields: Vec<FieldDiff>,
        notes: Vec<String>,
    ) {
        self.matches.insert(key.clone(), local.0.to_string());
        if fields.is_empty() {
            self.unchanged += 1;
            return;
        }
        self.items.push(ReviewItem {
            key,
            kind,
            name: name.into(),
            status: ItemStatus::Conflict,
            matched: Some(LocalMatch { id: local.0.into(), name: local.1.into() }),
            fields,
            notes,
        });
    }

    fn new(&mut self, key: String, kind: ItemKind, name: &str, notes: Vec<String>) {
        self.items.push(ReviewItem {
            key,
            kind,
            name: name.into(),
            status: ItemStatus::New,
            matched: None,
            fields: vec![],
            notes,
        });
    }

    fn group_name(&self, id: &str) -> String {
        self.local
            .groups
            .iter()
            .chain(&self.payload.groups)
            .find(|g| g.id == id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| id.into())
    }

    fn profile_name(&self, id: &str) -> String {
        self.local
            .profiles
            .iter()
            .chain(&self.payload.profiles)
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| id.into())
    }
}

// Where an incoming group reference lands locally: the matched group, a group arriving in the
// bundle, an identical local id, or nowhere (the profile imports ungrouped).
fn group_ref(
    id: Option<&str>,
    group_map: &HashMap<String, String>,
    payload: &Payload,
    local: &ProfileStore,
    notes: &mut Vec<String>,
) -> Option<String> {
    let id = id?;
    if let Some(mapped) = group_map.get(id) {
        return Some(mapped.clone());
    }
    if payload.groups.iter().any(|g| g.id == id) || local.groups.iter().any(|g| g.id == id) {
        return Some(id.to_string());
    }
    notes.push("Its group isn't in the export; it will be ungrouped".into());
    None
}

pub fn plan(local: &ProfileStore, payload: &Payload, env: &dyn Env) -> Plan {
    let mut c = Collector { items: vec![], unchanged: 0, matches: HashMap::new(), payload, local };

    let mut groups: Vec<&Group> = payload.groups.iter().collect();
    groups.sort_by_key(|g| group_depth(g, &payload.groups));
    let mut group_map: HashMap<String, String> = HashMap::new();
    let mut claimed = HashSet::new();
    for g in groups {
        let parent =
            g.parent_id.as_ref().map(|p| group_map.get(p).cloned().unwrap_or_else(|| p.clone()));
        let found = find_match(
            &local.groups,
            &mut claimed,
            |l| l.id.as_str(),
            |l| l.name.as_str(),
            |l| l.name == g.name && l.parent_id == parent,
            &g.id,
            &g.name,
        );
        let key = ItemKind::Group.key(&g.id);
        match found {
            Some(l) => {
                group_map.insert(g.id.clone(), l.id.clone());
                let (mut fields, mut notes) = (vec![], vec![]);
                push(&mut fields, "name", "Name", l.name.clone(), g.name.clone());
                if l.parent_id != parent {
                    fields.push(FieldDiff {
                        field: "parentId".into(),
                        label: "Parent group".into(),
                        local: l
                            .parent_id
                            .as_deref()
                            .map(|p| c.group_name(p))
                            .unwrap_or_else(|| "(top level)".into()),
                        incoming: parent
                            .as_deref()
                            .map(|p| c.group_name(p))
                            .unwrap_or_else(|| "(top level)".into()),
                    });
                }
                icon_field(&l.icon, &g.icon, env, &mut fields, &mut notes);
                c.matched(key, ItemKind::Group, &g.name, (&l.id, &l.name), fields, notes);
            }
            None => c.new(key, ItemKind::Group, &g.name, vec![]),
        }
    }

    let mut claimed = HashSet::new();
    let ssh: Vec<_> = payload
        .profiles
        .iter()
        .map(|p| {
            let mut p = p.clone();
            normalize_keys(&mut p);
            let found = find_match(
                &local.profiles,
                &mut claimed,
                |l| l.id.as_str(),
                |l| l.name.as_str(),
                |l| {
                    l.host.eq_ignore_ascii_case(&p.host)
                        && l.port == p.port
                        && l.username == p.username
                },
                &p.id,
                &p.name,
            );
            (p, found)
        })
        .collect();
    let profile_map: HashMap<String, String> =
        ssh.iter().filter_map(|(p, l)| l.map(|l| (p.id.clone(), l.id.clone()))).collect();
    for (inc, found) in &ssh {
        let key = ItemKind::Ssh.key(&inc.id);
        let (mut fields, mut notes) = (vec![], vec![]);
        let group = group_ref(inc.group_id.as_deref(), &group_map, payload, local, &mut notes);
        let jump = inc.jump_host_id.as_deref().and_then(|j| {
            if let Some(mapped) = profile_map.get(j) {
                return Some(mapped.clone());
            }
            if payload.profiles.iter().any(|p| p.id == j)
                || local.profiles.iter().any(|p| p.id == j)
            {
                return Some(j.to_string());
            }
            notes.push("Its jump host isn't in the export; it will connect directly".into());
            None
        });
        match found {
            Some(l) => {
                push(&mut fields, "name", "Name", l.name.clone(), inc.name.clone());
                push(&mut fields, "host", "Host", l.host.clone(), inc.host.clone());
                push(&mut fields, "port", "Port", l.port.to_string(), inc.port.to_string());
                push(&mut fields, "username", "Username", l.username.clone(), inc.username.clone());
                push(
                    &mut fields,
                    "authMethod",
                    "Login",
                    auth(l.auth_method),
                    auth(inc.auth_method),
                );
                if l.group_id != group {
                    fields.push(FieldDiff {
                        field: "groupId".into(),
                        label: "Group".into(),
                        local: l
                            .group_id
                            .as_deref()
                            .map(|g| c.group_name(g))
                            .unwrap_or_else(|| "(ungrouped)".into()),
                        incoming: group
                            .as_deref()
                            .map(|g| c.group_name(g))
                            .unwrap_or_else(|| "(ungrouped)".into()),
                    });
                }
                if l.jump_host_id != jump {
                    fields.push(FieldDiff {
                        field: "jumpHostId".into(),
                        label: "Jump host".into(),
                        local: text(
                            l.jump_host_id.as_deref().map(|j| c.profile_name(j)).as_deref(),
                        ),
                        incoming: text(jump.as_deref().map(|j| c.profile_name(j)).as_deref()),
                    });
                }
                icon_field(&l.icon, &inc.icon, env, &mut fields, &mut notes);
                push(
                    &mut fields,
                    "appearance",
                    "Appearance",
                    appearance_label(&l.appearance),
                    appearance_label(&inc.appearance),
                );
                let same_tunnels = l.tunnels.len() == inc.tunnels.len()
                    && l.tunnels.iter().zip(&inc.tunnels).all(|(a, b)| same_tunnel(a, b));
                if !same_tunnels {
                    fields.push(FieldDiff {
                        field: "tunnels".into(),
                        label: "Tunnels".into(),
                        local: tunnels_label(&l.tunnels),
                        incoming: tunnels_label(&inc.tunnels),
                    });
                }
                keys_field(
                    Some(l.keys.as_slice()),
                    &inc.keys,
                    payload,
                    env,
                    &mut fields,
                    &mut notes,
                );
                password_field(
                    "Password",
                    l.secret_id.as_deref(),
                    inc.secret_id.as_deref(),
                    payload,
                    env,
                    &mut fields,
                );
                c.matched(key, ItemKind::Ssh, &inc.name, (&l.id, &l.name), fields, notes);
            }
            None => {
                keys_field(None, &inc.keys, payload, env, &mut fields, &mut notes);
                if matches!(&inc.icon, IconRef::Custom { path } if !env.icon_exists(path)) {
                    notes.push(
                        "The custom icon isn't on this machine; using the default icon".into(),
                    );
                }
                c.new(key, ItemKind::Ssh, &inc.name, notes);
            }
        }
    }

    let mut claimed = HashSet::new();
    for inc in &payload.sftp_profiles {
        let key = ItemKind::Sftp.key(&inc.id);
        let found = find_match(
            &local.sftp_profiles,
            &mut claimed,
            |l| l.id.as_str(),
            |l| l.name.as_str(),
            |l| {
                l.host.eq_ignore_ascii_case(&inc.host)
                    && l.port == inc.port
                    && l.username == inc.username
            },
            &inc.id,
            &inc.name,
        );
        let (mut fields, mut notes) = (vec![], vec![]);
        match found {
            Some(l) => {
                push(&mut fields, "name", "Name", l.name.clone(), inc.name.clone());
                push(&mut fields, "host", "Host", l.host.clone(), inc.host.clone());
                push(&mut fields, "port", "Port", l.port.to_string(), inc.port.to_string());
                push(&mut fields, "username", "Username", l.username.clone(), inc.username.clone());
                push(
                    &mut fields,
                    "authMethod",
                    "Login",
                    auth(l.auth_method),
                    auth(inc.auth_method),
                );
                icon_field(&l.icon, &inc.icon, env, &mut fields, &mut notes);
                keys_field(
                    Some(l.keys.as_slice()),
                    &inc.keys,
                    payload,
                    env,
                    &mut fields,
                    &mut notes,
                );
                password_field(
                    "Password",
                    l.secret_id.as_deref(),
                    inc.secret_id.as_deref(),
                    payload,
                    env,
                    &mut fields,
                );
                c.matched(key, ItemKind::Sftp, &inc.name, (&l.id, &l.name), fields, notes);
            }
            None => {
                keys_field(None, &inc.keys, payload, env, &mut fields, &mut notes);
                c.new(key, ItemKind::Sftp, &inc.name, notes);
            }
        }
    }

    let mut claimed = HashSet::new();
    for inc in &payload.s3_profiles {
        let key = ItemKind::S3.key(&inc.id);
        let found = find_match(
            &local.s3_profiles,
            &mut claimed,
            |l| l.id.as_str(),
            |l| l.name.as_str(),
            |l| {
                l.endpoint.eq_ignore_ascii_case(&inc.endpoint)
                    && l.port == inc.port
                    && l.bucket == inc.bucket
                    && l.access_key_id == inc.access_key_id
            },
            &inc.id,
            &inc.name,
        );
        let (mut fields, mut notes) = (vec![], vec![]);
        match found {
            Some(l) => {
                push(&mut fields, "name", "Name", l.name.clone(), inc.name.clone());
                push(&mut fields, "endpoint", "Endpoint", l.endpoint.clone(), inc.endpoint.clone());
                push(
                    &mut fields,
                    "port",
                    "Port",
                    text(l.port.map(|p| p.to_string()).as_deref()),
                    text(inc.port.map(|p| p.to_string()).as_deref()),
                );
                push(&mut fields, "region", "Region", l.region.clone(), inc.region.clone());
                push(
                    &mut fields,
                    "useTls",
                    "Use TLS",
                    l.use_tls.to_string(),
                    inc.use_tls.to_string(),
                );
                push(
                    &mut fields,
                    "pathStyle",
                    "Path-style addressing",
                    l.path_style.to_string(),
                    inc.path_style.to_string(),
                );
                push(
                    &mut fields,
                    "accessKeyId",
                    "Access key id",
                    l.access_key_id.clone(),
                    inc.access_key_id.clone(),
                );
                push(
                    &mut fields,
                    "bucket",
                    "Bucket",
                    text(l.bucket.as_deref()),
                    text(inc.bucket.as_deref()),
                );
                icon_field(&l.icon, &inc.icon, env, &mut fields, &mut notes);
                password_field(
                    "Secret access key",
                    l.secret_id.as_deref(),
                    inc.secret_id.as_deref(),
                    payload,
                    env,
                    &mut fields,
                );
                c.matched(key, ItemKind::S3, &inc.name, (&l.id, &l.name), fields, notes);
            }
            None => c.new(key, ItemKind::S3, &inc.name, notes),
        }
    }

    Plan { items: c.items, unchanged: c.unchanged, matches: c.matches }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::{AuthMethod, Group, IconRef, Profile, ProfileKey};
    use crate::transfer::bundle::KeyFile;
    use crate::transfer::MapEnv;
    use zeroize::Zeroizing;

    fn profile(id: &str, name: &str, host: &str) -> Profile {
        Profile {
            id: id.into(),
            name: name.into(),
            group_id: None,
            host: host.into(),
            port: 22,
            username: "me".into(),
            auth_method: AuthMethod::Password,
            key_path: None,
            keys: vec![],
            secret_id: None,
            icon: IconRef::default(),
            order: 0,
            jump_host_id: None,
            tunnels: vec![],
            appearance: None,
        }
    }

    fn local(profiles: Vec<Profile>) -> ProfileStore {
        ProfileStore {
            version: 1,
            groups: vec![],
            profiles,
            sftp_profiles: vec![],
            s3_profiles: vec![],
        }
    }

    fn incoming(profiles: Vec<Profile>) -> Payload {
        Payload { profiles, ..Default::default() }
    }

    #[test]
    fn reimporting_own_export_is_all_unchanged() {
        let l = local(vec![profile("a", "web", "h1"), profile("b", "db", "h2")]);
        let p = plan(&l, &incoming(l.profiles.clone()), &MapEnv::default());
        assert!(p.items.is_empty());
        assert_eq!(p.unchanged, 2);
    }

    #[test]
    fn different_id_same_server_is_a_conflict_with_only_changed_fields() {
        let l = local(vec![profile("pc-id", "web", "h1")]);
        let mut mac = profile("mac-id", "web (mac)", "h1");
        mac.auth_method = AuthMethod::Agent;
        let p = plan(&l, &incoming(vec![mac]), &MapEnv::default());
        assert_eq!(p.items.len(), 1);
        let item = &p.items[0];
        assert_eq!(item.status, ItemStatus::Conflict);
        assert_eq!(item.matched.as_ref().unwrap().id, "pc-id");
        let fields: Vec<_> = item.fields.iter().map(|f| f.field.as_str()).collect();
        assert_eq!(fields, ["name", "authMethod"]);
        assert_eq!(p.matches.get("ssh:mac-id").map(String::as_str), Some("pc-id"));
    }

    #[test]
    fn ambiguous_same_server_without_name_match_is_new() {
        let l = local(vec![profile("a", "test", "h1"), profile("b", "test", "h1")]);
        let p = plan(&l, &incoming(vec![profile("x", "prod", "h1")]), &MapEnv::default());
        assert_eq!(p.items[0].status, ItemStatus::New);
    }

    #[test]
    fn ambiguous_same_server_prefers_the_same_name() {
        let l = local(vec![profile("a", "test", "h1"), profile("b", "prod", "h1")]);
        let mut inc = profile("x", "prod", "h1");
        inc.auth_method = AuthMethod::Agent;
        let p = plan(&l, &incoming(vec![inc]), &MapEnv::default());
        assert_eq!(p.items[0].matched.as_ref().unwrap().id, "b");
    }

    #[test]
    fn missing_key_path_keeps_local_keys_with_a_note() {
        let mut pc = profile("pc", "web", "h1");
        pc.auth_method = AuthMethod::Key;
        pc.keys = vec![ProfileKey { path: "C:\\keys\\id".into(), secret_id: None }];
        let mut mac = pc.clone();
        mac.id = "mac".into();
        mac.keys = vec![ProfileKey { path: "/Users/me/.ssh/id".into(), secret_id: None }];
        let mut env = MapEnv::default();
        env.files.insert("C:\\keys\\id".into(), b"PC".to_vec());
        let p = plan(&local(vec![pc]), &incoming(vec![mac]), &env);
        assert!(p.items.is_empty(), "keys kept, nothing else differs");
        assert_eq!(p.unchanged, 1);
    }

    #[test]
    fn embedded_key_with_different_content_is_a_keys_row() {
        let mut pc = profile("pc", "web", "h1");
        pc.auth_method = AuthMethod::Key;
        pc.keys = vec![ProfileKey { path: "C:\\keys\\id".into(), secret_id: None }];
        let mut mac = pc.clone();
        mac.id = "mac".into();
        mac.keys = vec![ProfileKey { path: "/Users/me/.ssh/id".into(), secret_id: None }];
        let mut payload = incoming(vec![mac.clone()]);
        payload.key_files.insert(
            "/Users/me/.ssh/id".into(),
            KeyFile { file_name: "id".into(), data: Zeroizing::new(STANDARD.encode(b"MAC")) },
        );
        let mut env = MapEnv::default();
        env.files.insert("C:\\keys\\id".into(), b"PC".to_vec());
        let p = plan(&local(vec![pc.clone()]), &payload, &env);
        assert_eq!(p.items[0].fields[0].field, "keys");

        // Same bytes under a different path: not a change.
        env.files.insert("C:\\keys\\id".into(), b"MAC".to_vec());
        assert!(plan(&local(vec![pc]), &payload, &env).items.is_empty());
    }

    #[test]
    fn password_row_only_when_bundle_carries_a_different_one() {
        let mut pc = profile("pc", "web", "h1");
        pc.secret_id = Some("local-sec".into());
        let mut mac = pc.clone();
        mac.id = "mac".into();
        mac.secret_id = Some("mac-sec".into());
        let mut env = MapEnv::default();
        env.secrets.insert("local-sec".into(), b"same".to_vec());

        assert!(plan(&local(vec![pc.clone()]), &incoming(vec![mac.clone()]), &env)
            .items
            .is_empty());

        let mut payload = incoming(vec![mac]);
        payload.secrets.insert("mac-sec".into(), Zeroizing::new("same".into()));
        assert!(plan(&local(vec![pc.clone()]), &payload, &env).items.is_empty());

        payload.secrets.insert("mac-sec".into(), Zeroizing::new("other".into()));
        let p = plan(&local(vec![pc]), &payload, &env);
        let row = &p.items[0].fields[0];
        assert_eq!(row.field, "password");
        assert!(!row.incoming.contains("other"));
    }

    #[test]
    fn groups_match_by_name_and_references_are_remapped() {
        let mut l = local(vec![profile("pc", "web", "h1")]);
        l.groups = vec![Group {
            id: "g-pc".into(),
            name: "Prod".into(),
            parent_id: None,
            icon: IconRef::default(),
            order: 0,
        }];
        l.profiles[0].group_id = Some("g-pc".into());
        let mut mac = profile("mac", "web", "h1");
        mac.group_id = Some("g-mac".into());
        let mut payload = incoming(vec![mac]);
        payload.groups = vec![Group {
            id: "g-mac".into(),
            name: "Prod".into(),
            parent_id: None,
            icon: IconRef::default(),
            order: 3,
        }];
        let p = plan(&l, &payload, &MapEnv::default());
        assert!(p.items.is_empty());
        assert_eq!(p.unchanged, 2);
        assert_eq!(p.matches.get("group:g-mac").map(String::as_str), Some("g-pc"));
    }

    #[test]
    fn dangling_group_is_noted_and_ungrouped() {
        let mut mac = profile("mac", "web", "h9");
        mac.group_id = Some("nowhere".into());
        let p = plan(&local(vec![]), &incoming(vec![mac]), &MapEnv::default());
        assert_eq!(p.items[0].status, ItemStatus::New);
        assert!(p.items[0].notes.iter().any(|n| n.contains("ungrouped")));
    }

    #[test]
    fn tunnel_ids_alone_are_not_a_change() {
        use crate::store::model::{Tunnel, TunnelKind};
        let t = |id: &str| Tunnel {
            id: id.into(),
            kind: TunnelKind::Local,
            bind_host: "127.0.0.1".into(),
            bind_port: 8080,
            target_host: Some("db".into()),
            target_port: Some(5432),
            auto_start: false,
        };
        let mut pc = profile("pc", "web", "h1");
        pc.tunnels = vec![t("t1")];
        let mut mac = pc.clone();
        mac.id = "mac".into();
        mac.tunnels = vec![t("t2")];
        assert!(plan(&local(vec![pc]), &incoming(vec![mac]), &MapEnv::default()).items.is_empty());
    }
}
