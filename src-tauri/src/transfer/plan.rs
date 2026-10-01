use std::collections::HashMap;

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

// Matches in passes so an earlier look-alike can't take the local item a later one matches
// outright: first by id, then the only free local on the same server with the same name, then
// the only free local on the same server. The keys are (id, name); `same_server` also gets the
// incoming id -> local id pairs matched so far.
fn match_items<'a, L, I>(
    locals: &'a [L],
    incoming: &[I],
    local_key: impl Fn(&L) -> (&str, &str),
    incoming_key: impl Fn(&I) -> (&str, &str),
    same_server: impl Fn(&L, &I, &HashMap<String, String>) -> bool,
) -> (Vec<Option<&'a L>>, HashMap<String, String>) {
    let mut found = vec![None; incoming.len()];
    let mut taken = vec![false; locals.len()];
    let mut map = HashMap::new();
    for (slot, inc) in found.iter_mut().zip(incoming) {
        let (id, _) = incoming_key(inc);
        let at = (0..locals.len()).find(|&at| !taken[at] && local_key(&locals[at]).0 == id);
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
        (notes, notes_as_new): (Vec<String>, Vec<String>),
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
            notes_as_new,
        });
    }

    fn added(&mut self, key: String, kind: ItemKind, name: &str, notes: Vec<String>) {
        self.items.push(ReviewItem {
            key,
            kind,
            name: name.into(),
            status: ItemStatus::New,
            matched: None,
            fields: vec![],
            notes_as_new: notes.clone(),
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
    let (found, group_map) = match_items(
        &local.groups,
        &groups,
        |l| (l.id.as_str(), l.name.as_str()),
        |g| (g.id.as_str(), g.name.as_str()),
        |l, g, map| {
            let parent = g.parent_id.as_ref().map(|p| map.get(p).unwrap_or(p));
            l.name == g.name && l.parent_id.as_ref() == parent
        },
    );
    for (g, found) in groups.into_iter().zip(found) {
        let parent =
            g.parent_id.as_ref().map(|p| group_map.get(p).cloned().unwrap_or_else(|| p.clone()));
        let key = ItemKind::Group.key(&g.id);
        let as_new = vec![];
        match found {
            Some(l) => {
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
                c.matched(key, ItemKind::Group, &g.name, (&l.id, &l.name), fields, (notes, as_new));
            }
            None => c.added(key, ItemKind::Group, &g.name, as_new),
        }
    }

    let ssh: Vec<_> = payload
        .profiles
        .iter()
        .map(|p| {
            let mut p = p.clone();
            normalize_keys(&mut p);
            p
        })
        .collect();
    let (found, profile_map) = match_items(
        &local.profiles,
        &ssh,
        |l| (l.id.as_str(), l.name.as_str()),
        |p| (p.id.as_str(), p.name.as_str()),
        |l, p, _| {
            l.host.eq_ignore_ascii_case(&p.host) && l.port == p.port && l.username == p.username
        },
    );
    for (inc, found) in ssh.iter().zip(found) {
        let key = ItemKind::Ssh.key(&inc.id);
        let (mut fields, mut notes) = (vec![], vec![]);
        let group = group_ref(inc.group_id.as_deref(), &group_map, payload, local, &mut notes);
        let mut as_new = notes.clone();
        let jump_ref = |same_jump: bool, notes: &mut Vec<String>| {
            inc.jump_host_id.as_deref().and_then(|j| {
                if same_jump {
                    return Some(j.to_string());
                }
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
            })
        };
        jump_ref(false, &mut as_new);
        keys_field(None, &inc.keys, payload, env, &mut vec![], &mut as_new);
        if matches!(&inc.icon, IconRef::Custom { path } if !env.icon_exists(path)) {
            as_new.push("The custom icon isn't on this machine; using the default icon".into());
        }
        match found {
            Some(l) => {
                // A jump host the import leaves alone is not a change, even if it points nowhere.
                let jump = jump_ref(l.jump_host_id == inc.jump_host_id, &mut notes);
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
                c.matched(key, ItemKind::Ssh, &inc.name, (&l.id, &l.name), fields, (notes, as_new));
            }
            None => c.added(key, ItemKind::Ssh, &inc.name, as_new),
        }
    }

    let (found, _) = match_items(
        &local.sftp_profiles,
        &payload.sftp_profiles,
        |l| (l.id.as_str(), l.name.as_str()),
        |p| (p.id.as_str(), p.name.as_str()),
        |l, p, _| {
            l.host.eq_ignore_ascii_case(&p.host) && l.port == p.port && l.username == p.username
        },
    );
    for (inc, found) in payload.sftp_profiles.iter().zip(found) {
        let key = ItemKind::Sftp.key(&inc.id);
        let (mut fields, mut notes) = (vec![], vec![]);
        let mut as_new = vec![];
        keys_field(None, &inc.keys, payload, env, &mut vec![], &mut as_new);
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
                c.matched(
                    key,
                    ItemKind::Sftp,
                    &inc.name,
                    (&l.id, &l.name),
                    fields,
                    (notes, as_new),
                );
            }
            None => c.added(key, ItemKind::Sftp, &inc.name, as_new),
        }
    }

    let (found, _) = match_items(
        &local.s3_profiles,
        &payload.s3_profiles,
        |l| (l.id.as_str(), l.name.as_str()),
        |p| (p.id.as_str(), p.name.as_str()),
        |l, p, _| {
            l.endpoint.eq_ignore_ascii_case(&p.endpoint)
                && l.port == p.port
                && l.bucket == p.bucket
                && l.access_key_id == p.access_key_id
        },
    );
    for (inc, found) in payload.s3_profiles.iter().zip(found) {
        let key = ItemKind::S3.key(&inc.id);
        let (mut fields, mut notes) = (vec![], vec![]);
        let as_new = vec![];
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
                c.matched(key, ItemKind::S3, &inc.name, (&l.id, &l.name), fields, (notes, as_new));
            }
            None => c.added(key, ItemKind::S3, &inc.name, as_new),
        }
    }

    Plan { items: c.items, unchanged: c.unchanged, matches: c.matches }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::{AuthMethod, Group, IconRef, Profile, ProfileKey, SftpProfile};
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
    fn missing_key_note_depends_on_whether_the_match_is_added_as_new() {
        let mut pc = profile("pc", "web", "h1");
        pc.auth_method = AuthMethod::Key;
        pc.keys = vec![ProfileKey { path: "C:\\keys\\id".into(), secret_id: None }];
        let mut mac = pc.clone();
        mac.id = "mac".into();
        mac.name = "web (mac)".into();
        mac.keys = vec![ProfileKey { path: "/Users/me/.ssh/id".into(), secret_id: None }];
        let mut env = MapEnv::default();
        env.files.insert("C:\\keys\\id".into(), b"PC".to_vec());
        let p = plan(&local(vec![pc]), &incoming(vec![mac]), &env);
        assert_eq!(p.items[0].status, ItemStatus::Conflict);
        assert_eq!(
            p.items[0].notes,
            ["Kept your local keys; not on this machine: /Users/me/.ssh/id"]
        );
        assert_eq!(p.items[0].notes_as_new, ["Key file not on this machine: /Users/me/.ssh/id"]);
    }

    #[test]
    fn matched_sftp_profile_gets_the_new_item_key_note_too() {
        let pc = SftpProfile {
            id: "pc".into(),
            name: "files".into(),
            host: "h1".into(),
            port: 22,
            username: "me".into(),
            auth_method: AuthMethod::Key,
            keys: vec![ProfileKey { path: "C:\\keys\\id".into(), secret_id: None }],
            secret_id: None,
            icon: IconRef::default(),
            order: 0,
        };
        let mac = SftpProfile {
            id: "mac".into(),
            name: "files (mac)".into(),
            keys: vec![ProfileKey { path: "/Users/me/.ssh/id".into(), secret_id: None }],
            ..pc.clone()
        };
        let l = ProfileStore { sftp_profiles: vec![pc], ..local(vec![]) };
        let payload = Payload { sftp_profiles: vec![mac], ..Default::default() };
        let p = plan(&l, &payload, &MapEnv::default());
        assert_eq!(
            p.items[0].notes,
            ["Kept your local keys; not on this machine: /Users/me/.ssh/id"]
        );
        assert_eq!(p.items[0].notes_as_new, ["Key file not on this machine: /Users/me/.ssh/id"]);
    }

    #[test]
    fn unmatched_item_has_the_same_notes_either_way() {
        let mut mac = profile("mac", "web", "h9");
        mac.group_id = Some("nowhere".into());
        mac.keys = vec![ProfileKey { path: "/Users/me/.ssh/id".into(), secret_id: None }];
        let p = plan(&local(vec![]), &incoming(vec![mac]), &MapEnv::default());
        assert_eq!(p.items[0].status, ItemStatus::New);
        assert_eq!(p.items[0].notes.len(), 2);
        assert_eq!(p.items[0].notes_as_new, p.items[0].notes);
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

    #[test]
    fn id_match_is_not_lost_to_an_earlier_same_server_item() {
        let l = local(vec![profile("x", "web", "h1")]);
        let bundle = incoming(vec![profile("a", "web", "h1"), profile("x", "web", "h1")]);
        let p = plan(&l, &bundle, &MapEnv::default());
        assert_eq!(p.matches.get("ssh:x").map(String::as_str), Some("x"));
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.items[0].key, "ssh:a");
        assert_eq!(p.items[0].status, ItemStatus::New);
        for item in p.items.iter().filter(|i| i.status == ItemStatus::New) {
            assert!(l.profiles.iter().all(|lp| ItemKind::Ssh.key(&lp.id) != item.key));
        }
    }

    #[test]
    fn same_name_match_is_not_lost_to_an_earlier_same_server_item() {
        let l = local(vec![profile("pc", "web", "h1")]);
        let bundle = incoming(vec![profile("m1", "foo", "h1"), profile("m2", "web", "h1")]);
        let p = plan(&l, &bundle, &MapEnv::default());
        assert_eq!(p.matches.get("ssh:m2").map(String::as_str), Some("pc"));
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.items[0].key, "ssh:m1");
        assert_eq!(p.items[0].status, ItemStatus::New);
    }

    #[test]
    fn nested_groups_match_through_the_mapped_parent() {
        let group = |id: &str, name: &str, parent: Option<&str>| Group {
            id: id.into(),
            name: name.into(),
            parent_id: parent.map(Into::into),
            icon: IconRef::default(),
            order: 0,
        };
        let mut l = local(vec![]);
        l.groups = vec![group("pc-prod", "Prod", None), group("pc-web", "Web", Some("pc-prod"))];
        let mut payload = incoming(vec![]);
        payload.groups =
            vec![group("mac-web", "Web", Some("mac-prod")), group("mac-prod", "Prod", None)];
        let p = plan(&l, &payload, &MapEnv::default());
        assert!(p.items.is_empty());
        assert_eq!(p.unchanged, 2);
        assert_eq!(p.matches.get("group:mac-prod").map(String::as_str), Some("pc-prod"));
        assert_eq!(p.matches.get("group:mac-web").map(String::as_str), Some("pc-web"));
    }

    #[test]
    fn untouched_dangling_jump_host_is_not_a_change() {
        let mut pc = profile("pc", "web", "h1");
        pc.jump_host_id = Some("gone".into());
        let l = local(vec![pc.clone()]);
        let p = plan(&l, &incoming(vec![pc.clone()]), &MapEnv::default());
        assert!(p.items.is_empty());
        assert_eq!(p.unchanged, 1);

        pc.name = "web 2".into();
        let p = plan(&l, &incoming(vec![pc]), &MapEnv::default());
        let fields: Vec<_> = p.items[0].fields.iter().map(|f| f.field.as_str()).collect();
        assert_eq!(fields, ["name"]);
        assert!(p.items[0].notes.is_empty());
        assert_eq!(
            p.items[0].notes_as_new,
            ["Its jump host isn't in the export; it will connect directly"]
        );
    }
}
