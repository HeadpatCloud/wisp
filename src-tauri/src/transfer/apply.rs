use std::collections::{HashMap, HashSet};
use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::bundle::{KeyFile, Payload};
use super::plan::{digest, group_depth, plan, same_tunnel, Plan};
use super::{ApplySummary, Env, ItemDecision, ItemKind, ItemProblem, ReviewItem};
use crate::error::{AppError, AppResult};
use crate::store::model::{AuthMethod, Group, IconRef, Profile, ProfileKey, ProfileStore, Tunnel};
use crate::store::{normalize_keys, Store};
use crate::vault::Vault;

pub const SECRET_PREFIX: &str = "pending-secret:";
pub const KEY_PREFIX: &str = "pending-key:";

pub struct Staged {
    pub data: ProfileStore,
    pub secrets: Vec<Zeroizing<String>>,
    pub key_files: Vec<KeyFile>,
    pub replaced_secrets: Vec<String>,
    pub summary: ApplySummary,
}

struct Stager<'a> {
    payload: &'a Payload,
    plan: &'a Plan,
    env: &'a dyn Env,
    decisions: HashMap<&'a str, &'a ItemDecision>,
    data: ProfileStore,
    secrets: Vec<Zeroizing<String>>,
    key_files: Vec<KeyFile>,
    replaced: Vec<String>,
    problems: Vec<ItemProblem>,
    summary: ApplySummary,
    group_ids: HashMap<String, Option<String>>,
    profile_ids: HashMap<String, Option<String>>,
    touched: Vec<(String, String)>,
}

impl<'a> Stager<'a> {
    fn decision(&self, key: &str) -> (bool, bool, HashSet<&'a str>) {
        let Some(d) = self.decisions.get(key) else { return (false, false, HashSet::new()) };
        // Only fields the plan offered for this item may be applied, whatever the caller sent.
        let offered = self.plan.items.iter().find(|i| i.key == key);
        let fields = d
            .fields
            .iter()
            .map(String::as_str)
            .filter(|f| offered.is_some_and(|i| i.fields.iter().any(|o| o.field == *f)))
            .collect();
        (d.accept, d.as_new, fields)
    }

    // Final local id for an incoming item, or None when it won't exist after apply.
    fn final_id(&self, kind: ItemKind, id: &str) -> Option<String> {
        let key = kind.key(id);
        let (accept, as_new, _) = self.decision(&key);
        // An incoming id that already exists locally must not be reused for a separate item.
        let taken = match kind {
            ItemKind::Group => self.data.groups.iter().any(|g| g.id == id),
            ItemKind::Ssh => self.data.profiles.iter().any(|p| p.id == id),
            ItemKind::Sftp => self.data.sftp_profiles.iter().any(|p| p.id == id),
            ItemKind::S3 => self.data.s3_profiles.iter().any(|p| p.id == id),
        };
        match self.plan.matches.get(&key) {
            Some(local) if !(accept && as_new) => Some(local.clone()),
            _ if accept => {
                Some(if as_new || taken { Uuid::new_v4().to_string() } else { id.to_string() })
            }
            _ => None,
        }
    }

    fn problem(&mut self, key: &str, message: String) {
        self.problems.push(ItemProblem { key: key.into(), message });
    }

    fn resolve(
        &self,
        map: &HashMap<String, Option<String>>,
        id: Option<&str>,
        exists_locally: bool,
        name: impl Fn(&str) -> String,
    ) -> Result<Option<String>, String> {
        let Some(id) = id else { return Ok(None) };
        match map.get(id) {
            Some(Some(final_id)) => Ok(Some(final_id.clone())),
            Some(None) => Err(name(id)),
            None if exists_locally => Ok(Some(id.to_string())),
            None => Ok(None),
        }
    }

    fn group(&self, id: Option<&str>) -> Result<Option<String>, String> {
        let local = id.is_some_and(|id| self.data.groups.iter().any(|g| g.id == id));
        self.resolve(&self.group_ids, id, local, |id| {
            let name = self
                .payload
                .groups
                .iter()
                .find(|g| g.id == id)
                .map(|g| g.name.as_str())
                .unwrap_or(id);
            format!("Its group \"{name}\" isn't being imported")
        })
    }

    fn jump(&self, id: Option<&str>) -> Result<Option<String>, String> {
        let local = id.is_some_and(|id| self.data.profiles.iter().any(|p| p.id == id));
        self.resolve(&self.profile_ids, id, local, |id| {
            let name = self
                .payload
                .profiles
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.name.as_str())
                .unwrap_or(id);
            format!("Its jump host \"{name}\" isn't being imported")
        })
    }

    fn secret(&mut self, incoming: Option<&str>) -> Option<String> {
        let value = incoming.and_then(|id| self.payload.secrets.get(id))?;
        self.secrets.push(value.clone());
        Some(format!("{SECRET_PREFIX}{}", self.secrets.len() - 1))
    }

    fn keys(&mut self, local: &[ProfileKey], incoming: &[ProfileKey]) -> Vec<ProfileKey> {
        let local_digests: Vec<(Option<String>, &ProfileKey)> =
            local.iter().map(|k| (self.env.read_file(&k.path).map(|b| digest(&b)), k)).collect();
        let mut out = Vec::new();
        for k in incoming {
            let embedded = self.payload.key_files.get(&k.path).cloned();
            // A key path on a network share is never stored: opening it at connect time would
            // send the user's credentials to whatever server the bundle named. Nor is a path
            // spelled like a placeholder, which would alias another item's embedded key file.
            if embedded.is_none()
                && (super::is_network_path(&k.path) || k.path.starts_with(KEY_PREFIX))
            {
                continue;
            }
            let content = match &embedded {
                Some(f) => STANDARD.decode(f.data.as_bytes()).ok().map(|b| digest(&b)),
                None => self.env.read_file(&k.path).map(|b| digest(&b)),
            };
            let path = match embedded {
                Some(f) => {
                    self.key_files.push(f);
                    format!("{KEY_PREFIX}{}", self.key_files.len() - 1)
                }
                None => k.path.clone(),
            };
            let secret_id = self.secret(k.secret_id.as_deref()).or_else(|| {
                local_digests
                    .iter()
                    .find(|(d, _)| d.is_some() && *d == content)
                    .and_then(|(_, l)| l.secret_id.clone())
            });
            out.push(ProfileKey { path, secret_id });
        }
        let kept: HashSet<&str> = out.iter().filter_map(|k| k.secret_id.as_deref()).collect();
        let replaced: Vec<String> = local
            .iter()
            .filter_map(|k| k.secret_id.clone())
            .filter(|id| !kept.contains(id.as_str()))
            .collect();
        self.replaced.extend(replaced);
        out
    }

    fn icon(&self, icon: &IconRef) -> IconRef {
        match icon {
            IconRef::Custom { path } if !self.env.icon_exists(path) => IconRef::default(),
            other => other.clone(),
        }
    }
}

fn fresh_tunnels(local: &[Tunnel], incoming: &[Tunnel]) -> Vec<Tunnel> {
    incoming
        .iter()
        .map(|t| Tunnel {
            id: local
                .iter()
                .find(|l| same_tunnel(l, t))
                .map(|l| l.id.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            ..t.clone()
        })
        .collect()
}

pub fn stage(
    local: &ProfileStore,
    payload: &Payload,
    plan: &Plan,
    decisions: &[ItemDecision],
    env: &dyn Env,
) -> Result<Staged, Vec<ItemProblem>> {
    let mut s = Stager {
        payload,
        plan,
        env,
        decisions: decisions.iter().map(|d| (d.key.as_str(), d)).collect(),
        data: local.clone(),
        secrets: vec![],
        key_files: vec![],
        replaced: vec![],
        problems: vec![],
        summary: ApplySummary { added: 0, updated: 0 },
        group_ids: HashMap::new(),
        profile_ids: HashMap::new(),
        touched: vec![],
    };
    for g in &payload.groups {
        let id = s.final_id(ItemKind::Group, &g.id);
        s.group_ids.insert(g.id.clone(), id);
    }
    for p in &payload.profiles {
        let id = s.final_id(ItemKind::Ssh, &p.id);
        s.profile_ids.insert(p.id.clone(), id);
    }

    let mut groups: Vec<&Group> = payload.groups.iter().collect();
    groups.sort_by_key(|g| group_depth(g, &payload.groups));
    for g in groups {
        let key = ItemKind::Group.key(&g.id);
        let (accept, as_new, fields) = s.decision(&key);
        if !accept {
            continue;
        }
        let matched = plan.matches.get(&key).filter(|_| !as_new).cloned();
        let parent = s.group(g.parent_id.as_deref());
        match matched {
            Some(local_id) => {
                if fields.is_empty() {
                    continue;
                }
                let mut next = s
                    .data
                    .groups
                    .iter()
                    .find(|x| x.id == local_id)
                    .cloned()
                    .expect("matched group exists");
                if fields.contains("name") {
                    next.name = g.name.clone();
                }
                if fields.contains("parentId") {
                    match parent {
                        Ok(p) => next.parent_id = p,
                        Err(m) => s.problem(&key, m),
                    }
                }
                if fields.contains("icon") {
                    next.icon = g.icon.clone();
                }
                if let Some(slot) = s.data.groups.iter_mut().find(|x| x.id == local_id) {
                    *slot = next;
                }
                s.summary.updated += 1;
                s.touched.push((key, local_id));
            }
            None => {
                let id =
                    s.group_ids.get(&g.id).cloned().flatten().expect("accepted group has an id");
                let parent_id = match parent {
                    Ok(p) => p,
                    Err(m) => {
                        s.problem(&key, m);
                        None
                    }
                };
                let order = s
                    .data
                    .groups
                    .iter()
                    .filter(|x| x.parent_id == parent_id)
                    .map(|x| x.order + 1)
                    .max()
                    .unwrap_or(0);
                let icon = s.icon(&g.icon);
                s.data.groups.push(Group {
                    id: id.clone(),
                    name: g.name.clone(),
                    parent_id,
                    icon,
                    order,
                });
                s.summary.added += 1;
                s.touched.push((key, id));
            }
        }
    }

    for p in &payload.profiles {
        let mut inc = p.clone();
        normalize_keys(&mut inc);
        let key = ItemKind::Ssh.key(&inc.id);
        let (accept, as_new, fields) = s.decision(&key);
        if !accept {
            continue;
        }
        let matched = plan.matches.get(&key).filter(|_| !as_new).cloned();
        match matched {
            Some(local_id) => {
                if fields.is_empty() {
                    continue;
                }
                let mut next = s
                    .data
                    .profiles
                    .iter()
                    .find(|x| x.id == local_id)
                    .cloned()
                    .expect("matched profile exists");
                for f in &fields {
                    match *f {
                        "name" => next.name = inc.name.clone(),
                        "host" => next.host = inc.host.clone(),
                        "port" => next.port = inc.port,
                        "username" => next.username = inc.username.clone(),
                        "authMethod" => next.auth_method = inc.auth_method,
                        "groupId" => match s.group(inc.group_id.as_deref()) {
                            Ok(g) => next.group_id = g,
                            Err(m) => s.problem(&key, m),
                        },
                        "jumpHostId" => match s.jump(inc.jump_host_id.as_deref()) {
                            Ok(j) => next.jump_host_id = j,
                            Err(m) => s.problem(&key, m),
                        },
                        "icon" => next.icon = inc.icon.clone(),
                        "appearance" => next.appearance = inc.appearance.clone(),
                        "tunnels" => next.tunnels = fresh_tunnels(&next.tunnels, &inc.tunnels),
                        "keys" => next.keys = s.keys(&next.keys.clone(), &inc.keys),
                        "password" => {
                            s.replaced.extend(next.secret_id.take());
                            next.secret_id = s.secret(inc.secret_id.as_deref());
                        }
                        _ => {}
                    }
                }
                if let Some(slot) = s.data.profiles.iter_mut().find(|x| x.id == local_id) {
                    *slot = next;
                }
                s.summary.updated += 1;
                s.touched.push((key, local_id));
            }
            None => {
                let id = s
                    .profile_ids
                    .get(&inc.id)
                    .cloned()
                    .flatten()
                    .expect("accepted profile has an id");
                let group_id = s.group(inc.group_id.as_deref()).unwrap_or_else(|m| {
                    s.problem(&key, m);
                    None
                });
                let jump_host_id = s.jump(inc.jump_host_id.as_deref()).unwrap_or_else(|m| {
                    s.problem(&key, m);
                    None
                });
                let keys = s.keys(&[], &inc.keys);
                let secret_id = s.secret(inc.secret_id.as_deref());
                let order = s
                    .data
                    .profiles
                    .iter()
                    .filter(|x| x.group_id == group_id)
                    .map(|x| x.order + 1)
                    .max()
                    .unwrap_or(0);
                let profile = Profile {
                    id: id.clone(),
                    group_id,
                    jump_host_id,
                    keys,
                    secret_id,
                    icon: s.icon(&inc.icon),
                    tunnels: fresh_tunnels(&[], &inc.tunnels),
                    order,
                    ..inc
                };
                s.data.profiles.push(profile);
                s.summary.added += 1;
                s.touched.push((key, id));
            }
        }
    }

    for inc in &payload.sftp_profiles {
        let key = ItemKind::Sftp.key(&inc.id);
        let (accept, as_new, fields) = s.decision(&key);
        if !accept {
            continue;
        }
        match plan.matches.get(&key).filter(|_| !as_new).cloned() {
            Some(local_id) => {
                if fields.is_empty() {
                    continue;
                }
                let mut next = s
                    .data
                    .sftp_profiles
                    .iter()
                    .find(|x| x.id == local_id)
                    .cloned()
                    .expect("matched sftp profile exists");
                for f in &fields {
                    match *f {
                        "name" => next.name = inc.name.clone(),
                        "host" => next.host = inc.host.clone(),
                        "port" => next.port = inc.port,
                        "username" => next.username = inc.username.clone(),
                        "authMethod" => next.auth_method = inc.auth_method,
                        "icon" => next.icon = inc.icon.clone(),
                        "keys" => next.keys = s.keys(&next.keys.clone(), &inc.keys),
                        "password" => {
                            s.replaced.extend(next.secret_id.take());
                            next.secret_id = s.secret(inc.secret_id.as_deref());
                        }
                        _ => {}
                    }
                }
                if let Some(slot) = s.data.sftp_profiles.iter_mut().find(|x| x.id == local_id) {
                    *slot = next;
                }
                s.summary.updated += 1;
                s.touched.push((key, local_id));
            }
            None => {
                let id =
                    s.final_id(ItemKind::Sftp, &inc.id).expect("accepted sftp profile has an id");
                let keys = s.keys(&[], &inc.keys);
                let secret_id = s.secret(inc.secret_id.as_deref());
                let order = s.data.sftp_profiles.iter().map(|x| x.order + 1).max().unwrap_or(0);
                let icon = s.icon(&inc.icon);
                s.data.sftp_profiles.push(crate::store::model::SftpProfile {
                    id: id.clone(),
                    keys,
                    secret_id,
                    icon,
                    order,
                    ..inc.clone()
                });
                s.summary.added += 1;
                s.touched.push((key, id));
            }
        }
    }

    for inc in &payload.s3_profiles {
        let key = ItemKind::S3.key(&inc.id);
        let (accept, as_new, fields) = s.decision(&key);
        if !accept {
            continue;
        }
        match plan.matches.get(&key).filter(|_| !as_new).cloned() {
            Some(local_id) => {
                if fields.is_empty() {
                    continue;
                }
                let mut next = s
                    .data
                    .s3_profiles
                    .iter()
                    .find(|x| x.id == local_id)
                    .cloned()
                    .expect("matched s3 profile exists");
                for f in &fields {
                    match *f {
                        "name" => next.name = inc.name.clone(),
                        "endpoint" => next.endpoint = inc.endpoint.clone(),
                        "port" => next.port = inc.port,
                        "region" => next.region = inc.region.clone(),
                        "useTls" => next.use_tls = inc.use_tls,
                        "pathStyle" => next.path_style = inc.path_style,
                        "accessKeyId" => next.access_key_id = inc.access_key_id.clone(),
                        "bucket" => next.bucket = inc.bucket.clone(),
                        "icon" => next.icon = inc.icon.clone(),
                        "password" => {
                            s.replaced.extend(next.secret_id.take());
                            next.secret_id = s.secret(inc.secret_id.as_deref());
                        }
                        _ => {}
                    }
                }
                if let Some(slot) = s.data.s3_profiles.iter_mut().find(|x| x.id == local_id) {
                    *slot = next;
                }
                s.summary.updated += 1;
                s.touched.push((key, local_id));
            }
            None => {
                let id = s.final_id(ItemKind::S3, &inc.id).expect("accepted s3 profile has an id");
                let secret_id = s.secret(inc.secret_id.as_deref());
                let order = s.data.s3_profiles.iter().map(|x| x.order + 1).max().unwrap_or(0);
                let icon = s.icon(&inc.icon);
                s.data.s3_profiles.push(crate::store::model::S3Profile {
                    id: id.clone(),
                    secret_id,
                    icon,
                    order,
                    ..inc.clone()
                });
                s.summary.added += 1;
                s.touched.push((key, id));
            }
        }
    }

    let touched = std::mem::take(&mut s.touched);
    let mut found: Vec<(String, String)> = Vec::new();
    for (key, id) in &touched {
        if let Some(p) = s.data.profiles.iter().find(|p| &p.id == id) {
            if p.auth_method == AuthMethod::Key && p.keys.is_empty() {
                found.push((key.clone(), "Key login needs at least one private key".into()));
            }
            let mut seen = HashSet::new();
            let mut hop = p.jump_host_id.clone();
            while let Some(h) = hop {
                if &h == id || !seen.insert(h.clone()) {
                    found.push((key.clone(), "Its jump hosts would form a loop".into()));
                    break;
                }
                hop =
                    s.data.profiles.iter().find(|x| x.id == h).and_then(|x| x.jump_host_id.clone());
            }
        }
        if let Some(p) = s.data.sftp_profiles.iter().find(|p| &p.id == id) {
            if p.auth_method == AuthMethod::Key && p.keys.is_empty() {
                found.push((key.clone(), "Key login needs at least one private key".into()));
            }
        }
        if let Some(g) = s.data.groups.iter().find(|g| &g.id == id) {
            let mut seen = HashSet::new();
            let mut parent = g.parent_id.clone();
            while let Some(pid) = parent {
                if &pid == id || !seen.insert(pid.clone()) {
                    found.push((key.clone(), "Its parent groups would form a loop".into()));
                    break;
                }
                parent =
                    s.data.groups.iter().find(|g| g.id == pid).and_then(|g| g.parent_id.clone());
            }
        }
    }
    for (key, message) in found {
        s.problem(&key, message);
    }

    if !s.problems.is_empty() {
        return Err(s.problems);
    }
    Ok(Staged {
        data: s.data,
        secrets: s.secrets,
        key_files: s.key_files,
        replaced_secrets: s.replaced,
        summary: s.summary,
    })
}

pub fn stage_reviewed(
    local: &ProfileStore,
    payload: &Payload,
    reviewed: &[ReviewItem],
    decisions: &[ItemDecision],
    env: &dyn Env,
) -> AppResult<Result<Staged, Vec<ItemProblem>>> {
    let planned = plan(local, payload, env);
    // The decisions were made on the stored review; on a different plan they could land on a
    // profile the user never saw.
    if planned.items != reviewed {
        return Err(AppError::Import(
            "your profiles changed since this file was opened; close this tab and import the \
             file again"
                .into(),
        ));
    }
    Ok(stage(local, payload, &planned, decisions, env))
}

fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .take(64)
        .collect();
    if cleaned.trim_matches('.').is_empty() {
        "key".into()
    } else {
        cleaned
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> AppResult<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    f.write_all(bytes)?;
    Ok(())
}

// Under the user's AppData the file inherits the profile's user/SYSTEM/Administrators-only ACL.
#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> AppResult<()> {
    std::fs::write(path, bytes)?;
    Ok(())
}

fn placeholder(value: &str, prefix: &str, real: &[String]) -> Option<String> {
    value
        .strip_prefix(prefix)
        .and_then(|i| i.parse::<usize>().ok())
        .and_then(|i| real.get(i).cloned())
}

fn substitute(data: &mut ProfileStore, secrets: &[String], keys: &[String]) {
    let fix = |id: &mut Option<String>| {
        if let Some(real) = id.as_deref().and_then(|v| placeholder(v, SECRET_PREFIX, secrets)) {
            *id = Some(real);
        }
    };
    for p in data.profiles.iter_mut() {
        fix(&mut p.secret_id);
        for k in p.keys.iter_mut() {
            fix(&mut k.secret_id);
            if let Some(real) = placeholder(&k.path, KEY_PREFIX, keys) {
                k.path = real;
            }
        }
    }
    for p in data.sftp_profiles.iter_mut() {
        fix(&mut p.secret_id);
        for k in p.keys.iter_mut() {
            fix(&mut k.secret_id);
            if let Some(real) = placeholder(&k.path, KEY_PREFIX, keys) {
                k.path = real;
            }
        }
    }
    for p in data.s3_profiles.iter_mut() {
        fix(&mut p.secret_id);
    }
}

pub fn execute(
    store: &mut Store,
    vault: &mut Vault,
    keys_dir: &Path,
    staged: Staged,
) -> AppResult<ApplySummary> {
    let Staged { mut data, secrets, key_files, replaced_secrets, summary } = staged;
    let mut written = Vec::new();
    let mut new_secrets = Vec::new();
    let result = (|| -> AppResult<()> {
        let mut key_paths = Vec::new();
        if !key_files.is_empty() {
            std::fs::create_dir_all(keys_dir)?;
        }
        for f in &key_files {
            let bytes = Zeroizing::new(
                STANDARD
                    .decode(f.data.as_bytes())
                    .map_err(|_| AppError::Import("a key file in the export is corrupt".into()))?,
            );
            let path = keys_dir.join(format!("{}-{}", Uuid::new_v4(), safe_name(&f.file_name)));
            // Recorded first so a write that fails midway is still removed by the rollback.
            written.push(path.clone());
            write_private(&path, &bytes)?;
            key_paths.push(path.to_string_lossy().into_owned());
        }
        for value in &secrets {
            new_secrets.push(vault.set_secret(value.as_bytes())?);
        }
        substitute(&mut data, &new_secrets, &key_paths);
        store.commit(data)
    })();
    if let Err(e) = result {
        // Undo everything this apply created; the store itself was never swapped.
        for path in &written {
            let _ = std::fs::remove_file(path);
        }
        for id in &new_secrets {
            let _ = vault.delete_secret(id);
        }
        return Err(e);
    }
    for id in &replaced_secrets {
        let _ = vault.delete_secret(id);
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::{AuthMethod, Group, IconRef, Profile, ProfileKey};
    use crate::transfer::bundle::KeyFile;
    use crate::transfer::plan::plan;
    use crate::transfer::{LocalEnv, MapEnv};
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

    fn store_of(profiles: Vec<Profile>) -> ProfileStore {
        ProfileStore {
            version: 1,
            groups: vec![],
            profiles,
            sftp_profiles: vec![],
            s3_profiles: vec![],
        }
    }

    fn group(id: &str, name: &str, parent: Option<&str>) -> Group {
        Group {
            id: id.into(),
            name: name.into(),
            parent_id: parent.map(Into::into),
            icon: IconRef::default(),
            order: 0,
        }
    }

    fn accept(key: &str, fields: &[&str]) -> ItemDecision {
        ItemDecision {
            key: key.into(),
            accept: true,
            as_new: false,
            fields: fields.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn run(
        local: &ProfileStore,
        payload: &Payload,
        decisions: &[ItemDecision],
    ) -> Result<Staged, Vec<ItemProblem>> {
        let env = MapEnv::default();
        stage(local, payload, &plan(local, payload, &env), decisions, &env)
    }

    #[test]
    fn accepts_only_the_chosen_fields() {
        let local = store_of(vec![profile("pc", "web", "h1")]);
        let mut mac = profile("mac", "web (mac)", "h1");
        mac.auth_method = AuthMethod::Agent;
        let payload = Payload { profiles: vec![mac], ..Default::default() };
        let staged = run(&local, &payload, &[accept("ssh:mac", &["authMethod"])]).unwrap();
        let p = &staged.data.profiles[0];
        assert_eq!(
            (p.id.as_str(), p.name.as_str(), p.auth_method),
            ("pc", "web", AuthMethod::Agent)
        );
        assert_eq!(staged.summary, ApplySummary { added: 0, updated: 1 });
    }

    #[test]
    fn fields_the_plan_did_not_offer_are_ignored() {
        let local = store_of(vec![profile("pc", "web", "h1")]);
        let mut mac = profile("mac", "web (mac)", "h1");
        mac.icon = IconRef::Custom { path: "C:\\Windows\\win.ini".into() };
        let payload = Payload { profiles: vec![mac], ..Default::default() };
        let staged =
            run(&local, &payload, &[accept("ssh:mac", &["name", "icon", "host"])]).unwrap();
        let p = &staged.data.profiles[0];
        assert_eq!(p.name, "web (mac)");
        assert_eq!(p.icon, IconRef::default());
    }

    #[test]
    fn declined_items_leave_everything_untouched() {
        let local = store_of(vec![profile("pc", "web", "h1")]);
        let mut mac = profile("mac", "web", "h1");
        mac.port = 2222;
        let payload =
            Payload { profiles: vec![mac, profile("n", "new", "h9")], ..Default::default() };
        let staged = run(&local, &payload, &[]).unwrap();
        assert_eq!(staged.data, local);
    }

    #[test]
    fn add_as_new_creates_a_separate_profile() {
        let local = store_of(vec![profile("pc", "web", "h1")]);
        let mut mac = profile("pc", "web", "h1");
        mac.port = 2222;
        let payload = Payload { profiles: vec![mac], ..Default::default() };
        let d = ItemDecision { key: "ssh:pc".into(), accept: true, as_new: true, fields: vec![] };
        let staged = run(&local, &payload, &[d]).unwrap();
        assert_eq!(staged.data.profiles.len(), 2);
        assert_ne!(staged.data.profiles[1].id, "pc");
        assert_eq!(staged.data.profiles[0].port, 22);
    }

    #[test]
    fn new_profile_gets_bundle_secrets_and_embedded_keys_as_placeholders() {
        let mut inc = profile("n", "new", "h9");
        inc.auth_method = AuthMethod::Key;
        inc.secret_id = Some("mac-pw".into());
        inc.keys =
            vec![ProfileKey { path: "/Users/me/id".into(), secret_id: Some("mac-pp".into()) }];
        let mut payload = Payload { profiles: vec![inc], ..Default::default() };
        payload.secrets.insert("mac-pw".into(), Zeroizing::new("pw".into()));
        payload.secrets.insert("mac-pp".into(), Zeroizing::new("pp".into()));
        payload.key_files.insert(
            "/Users/me/id".into(),
            KeyFile { file_name: "id".into(), data: Zeroizing::new(STANDARD.encode(b"K")) },
        );
        let staged = run(&store_of(vec![]), &payload, &[accept("ssh:n", &[])]).unwrap();
        let p = &staged.data.profiles[0];
        assert!(p.secret_id.as_deref().unwrap().starts_with(SECRET_PREFIX));
        assert!(p.keys[0].path.starts_with(KEY_PREFIX));
        assert!(p.keys[0].secret_id.as_deref().unwrap().starts_with(SECRET_PREFIX));
        assert_eq!(staged.secrets.len(), 2);
        assert_eq!(staged.key_files.len(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn network_key_paths_are_never_stored() {
        let mut inc = profile("n", "new", "h9");
        inc.keys = [
            "\\\\evil\\share\\id",
            "\\??\\UNC\\evil\\share\\id",
            "\\??\\GLOBALROOT\\Device\\Mup\\evil\\share\\id",
        ]
        .map(|path| ProfileKey { path: path.into(), secret_id: None })
        .into();
        let payload = Payload { profiles: vec![inc.clone()], ..Default::default() };
        let staged = run(&store_of(vec![]), &payload, &[accept("ssh:n", &[])]).unwrap();
        assert!(staged.data.profiles[0].keys.is_empty());

        inc.auth_method = AuthMethod::Key;
        let payload = Payload { profiles: vec![inc], ..Default::default() };
        let problems = run(&store_of(vec![]), &payload, &[accept("ssh:n", &[])]).err().unwrap();
        assert!(problems[0].message.contains("private key"));
    }

    #[test]
    fn placeholder_key_paths_are_never_stored() {
        let mut inc = profile("n", "new", "h9");
        inc.keys = vec![ProfileKey { path: format!("{KEY_PREFIX}0"), secret_id: None }];
        let payload = Payload { profiles: vec![inc], ..Default::default() };
        let staged = run(&store_of(vec![]), &payload, &[accept("ssh:n", &[])]).unwrap();
        assert!(staged.data.profiles[0].keys.is_empty());
        assert!(staged.key_files.is_empty());
    }

    #[test]
    fn replacing_a_password_schedules_the_old_secret_for_removal() {
        let mut pc = profile("pc", "web", "h1");
        pc.secret_id = Some("old".into());
        let mut mac = pc.clone();
        mac.id = "mac".into();
        mac.secret_id = Some("mac-pw".into());
        let mut payload = Payload { profiles: vec![mac], ..Default::default() };
        payload.secrets.insert("mac-pw".into(), Zeroizing::new("new".into()));
        let local = store_of(vec![pc]);
        let staged = run(&local, &payload, &[accept("ssh:mac", &["password"])]).unwrap();
        assert_eq!(staged.replaced_secrets, ["old"]);
    }

    #[test]
    fn key_login_without_keys_is_a_problem() {
        let mut pc = profile("pc", "web", "h1");
        pc.auth_method = AuthMethod::Password;
        let mut mac = pc.clone();
        mac.id = "mac".into();
        mac.auth_method = AuthMethod::Key;
        let payload = Payload { profiles: vec![mac], ..Default::default() };
        let problems = run(&store_of(vec![pc]), &payload, &[accept("ssh:mac", &["authMethod"])])
            .err()
            .unwrap();
        assert_eq!(problems[0].key, "ssh:mac");
    }

    #[test]
    fn declined_new_group_blocks_a_profile_that_moves_into_it() {
        let mut inc = profile("n", "new", "h9");
        inc.group_id = Some("g".into());
        let payload = Payload {
            groups: vec![Group {
                id: "g".into(),
                name: "Lab".into(),
                parent_id: None,
                icon: IconRef::default(),
                order: 0,
            }],
            profiles: vec![inc],
            ..Default::default()
        };
        let problems = run(&store_of(vec![]), &payload, &[accept("ssh:n", &[])]).err().unwrap();
        assert!(problems[0].message.contains("Lab"));
        assert!(run(&store_of(vec![]), &payload, &[accept("ssh:n", &[]), accept("group:g", &[])])
            .is_ok());
    }

    #[test]
    fn matched_group_declined_as_new_keeps_its_local_id() {
        let local = ProfileStore { groups: vec![group("g-pc", "Lab", None)], ..store_of(vec![]) };
        let mut inc = profile("n", "new", "h9");
        inc.group_id = Some("g-mac".into());
        let payload = Payload {
            groups: vec![group("g-mac", "Lab", None)],
            profiles: vec![inc],
            ..Default::default()
        };
        let declined =
            ItemDecision { key: "group:g-mac".into(), accept: false, as_new: true, fields: vec![] };
        let staged = run(&local, &payload, &[declined, accept("ssh:n", &[])]).unwrap();
        assert_eq!(staged.data.profiles[0].group_id.as_deref(), Some("g-pc"));
    }

    #[test]
    fn declined_new_jump_host_blocks_its_user() {
        let mut web = profile("w", "web", "h1");
        web.jump_host_id = Some("b".into());
        let payload =
            Payload { profiles: vec![web, profile("b", "bastion", "h2")], ..Default::default() };
        let problems = run(&store_of(vec![]), &payload, &[accept("ssh:w", &[])]).err().unwrap();
        assert!(problems[0].message.contains("bastion"));
    }

    #[test]
    fn dangling_group_imports_ungrouped() {
        let mut inc = profile("n", "new", "h9");
        inc.group_id = Some("nowhere".into());
        let payload = Payload { profiles: vec![inc], ..Default::default() };
        let staged = run(&store_of(vec![]), &payload, &[accept("ssh:n", &[])]).unwrap();
        assert_eq!(staged.data.profiles[0].group_id, None);
    }

    #[test]
    fn jump_host_loop_is_a_problem() {
        let a = profile("a", "a", "ha");
        let mut b = profile("b", "b", "hb");
        b.jump_host_id = Some("a".into());
        let mut inc = a.clone();
        inc.jump_host_id = Some("b".into());
        let payload = Payload { profiles: vec![inc], ..Default::default() };
        let problems = run(&store_of(vec![a, b]), &payload, &[accept("ssh:a", &["jumpHostId"])])
            .err()
            .unwrap();
        assert!(problems[0].message.contains("jump hosts would form a loop"));
    }

    #[test]
    fn parent_group_loop_is_a_problem() {
        let local = ProfileStore {
            groups: vec![group("g1", "one", None), group("g2", "two", Some("g1"))],
            ..store_of(vec![])
        };
        let payload =
            Payload { groups: vec![group("g1", "one", Some("g2"))], ..Default::default() };
        let problems = run(&local, &payload, &[accept("group:g1", &["parentId"])]).err().unwrap();
        assert!(problems[0].message.contains("parent groups would form a loop"));
    }

    fn vault(dir: &std::path::Path) -> Vault {
        Vault::open_with_key(dir.join("vault.enc"), Zeroizing::new([3u8; 32])).unwrap()
    }

    fn staged_with_secret_and_key() -> Staged {
        let mut inc = profile("n", "new", "h9");
        inc.auth_method = AuthMethod::Key;
        inc.secret_id = Some("mac-pw".into());
        inc.keys = vec![ProfileKey { path: "/Users/me/id".into(), secret_id: None }];
        let mut payload = Payload { profiles: vec![inc], ..Default::default() };
        payload.secrets.insert("mac-pw".into(), Zeroizing::new("pw".into()));
        payload.key_files.insert(
            "/Users/me/id".into(),
            KeyFile { file_name: "id".into(), data: Zeroizing::new(STANDARD.encode(b"K")) },
        );
        run(&store_of(vec![]), &payload, &[accept("ssh:n", &[])]).unwrap()
    }

    #[test]
    fn icon_outside_the_icons_folder_becomes_the_default_icon() {
        let dir = tempfile::tempdir().unwrap();
        let v = vault(dir.path());
        std::fs::write(dir.path().join("vault.enc"), b"x").unwrap();
        let env = LocalEnv { vault: &v, config_dir: dir.path() };
        let icon = IconRef::Custom { path: "vault.enc".into() };
        let mut inc = profile("n", "new", "h9");
        inc.icon = icon.clone();
        let payload = Payload {
            groups: vec![Group { icon, ..group("g", "Lab", None) }],
            profiles: vec![inc],
            ..Default::default()
        };
        let local = store_of(vec![]);
        let decisions = [accept("group:g", &[]), accept("ssh:n", &[])];
        let staged =
            stage(&local, &payload, &plan(&local, &payload, &env), &decisions, &env).unwrap();
        assert_eq!(staged.data.groups[0].icon, IconRef::default());
        assert_eq!(staged.data.profiles[0].icon, IconRef::default());
    }

    #[test]
    fn execute_writes_secrets_and_keys_then_commits() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        let mut v = vault(dir.path());
        let summary =
            execute(&mut store, &mut v, &dir.path().join("keys"), staged_with_secret_and_key())
                .unwrap();
        assert_eq!(summary, ApplySummary { added: 1, updated: 0 });
        let p = &store.profiles()[0];
        assert_eq!(v.get_secret(p.secret_id.as_deref().unwrap()).unwrap().as_slice(), b"pw");
        assert_eq!(std::fs::read(&p.keys[0].path).unwrap(), b"K");
        assert!(p.keys[0].path.starts_with(dir.path().join("keys").to_string_lossy().as_ref()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p.keys[0].path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn failed_commit_rolls_back_secrets_and_key_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        std::fs::create_dir(dir.path().join("profiles.json")).unwrap();
        let mut v = vault(dir.path());
        let keys_dir = dir.path().join("keys");
        assert!(execute(&mut store, &mut v, &keys_dir, staged_with_secret_and_key()).is_err());
        assert!(store.profiles().is_empty());
        assert_eq!(std::fs::read_dir(&keys_dir).unwrap().count(), 0);
        let raw = std::fs::read_to_string(dir.path().join("vault.enc")).unwrap_or_default();
        let secrets = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|v| v["secrets"].as_object().map(|o| o.len()))
            .unwrap_or(0);
        assert_eq!(secrets, 0);
    }

    #[test]
    fn corrupt_key_file_rolls_back_the_one_already_written() {
        let mut inc = profile("n", "new", "h9");
        inc.keys = vec![
            ProfileKey { path: "/a".into(), secret_id: None },
            ProfileKey { path: "/b".into(), secret_id: None },
        ];
        let mut payload = Payload { profiles: vec![inc], ..Default::default() };
        let good = KeyFile { file_name: "a".into(), data: Zeroizing::new(STANDARD.encode(b"K")) };
        let bad = KeyFile { file_name: "b".into(), data: Zeroizing::new("not base64!".into()) };
        payload.key_files.insert("/a".into(), good);
        payload.key_files.insert("/b".into(), bad);
        let staged = run(&store_of(vec![]), &payload, &[accept("ssh:n", &[])]).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        let mut v = vault(dir.path());
        let keys_dir = dir.path().join("keys");
        let result = execute(&mut store, &mut v, &keys_dir, staged);
        assert!(matches!(result, Err(AppError::Import(_))));
        assert_eq!(std::fs::read_dir(&keys_dir).unwrap().count(), 0);
        assert!(store.profiles().is_empty());
    }

    #[test]
    fn locked_vault_rolls_back_key_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        let mut v = Vault::open_locked(dir.path().join("vault.enc")).unwrap();
        let keys_dir = dir.path().join("keys");
        let result = execute(&mut store, &mut v, &keys_dir, staged_with_secret_and_key());
        assert!(matches!(result, Err(AppError::Vault(_))));
        assert_eq!(std::fs::read_dir(&keys_dir).unwrap().count(), 0);
        assert!(store.profiles().is_empty());
    }

    // A stored profile whose password is in the vault, and an import that replaces that password.
    fn password_replacement(dir: &std::path::Path) -> (Store, Vault, String, Staged) {
        let mut store = Store::load(dir.to_path_buf()).unwrap();
        let mut v = vault(dir);
        let old = v.set_secret(b"old").unwrap();
        let mut pc = profile("pc", "web", "h1");
        pc.secret_id = Some(old.clone());
        store.commit(store_of(vec![pc.clone()])).unwrap();
        let mut mac = pc;
        mac.id = "mac".into();
        mac.secret_id = Some("mac-pw".into());
        let mut payload = Payload { profiles: vec![mac], ..Default::default() };
        payload.secrets.insert("mac-pw".into(), Zeroizing::new("new".into()));
        let staged = run(&store.snapshot(), &payload, &[accept("ssh:mac", &["password"])]).unwrap();
        (store, v, old, staged)
    }

    // Two local profiles on one server, and a review of an incoming profile that matched "web".
    fn reviewed_import(dir: &std::path::Path) -> (Store, Vault, String, Payload, Vec<ReviewItem>) {
        let mut store = Store::load(dir.to_path_buf()).unwrap();
        let mut v = vault(dir);
        let old = v.set_secret(b"old").unwrap();
        let mut web = profile("w", "web", "h1");
        web.secret_id = Some(old.clone());
        store.commit(store_of(vec![web, profile("d", "db", "h1")])).unwrap();
        let mut mac = profile("mac", "web", "h1");
        mac.auth_method = AuthMethod::Agent;
        mac.secret_id = Some("mac-pw".into());
        let mut payload = Payload { profiles: vec![mac], ..Default::default() };
        payload.secrets.insert("mac-pw".into(), Zeroizing::new("new".into()));
        let items =
            plan(&store.snapshot(), &payload, &LocalEnv { vault: &v, config_dir: dir }).items;
        assert_eq!(items[0].matched.as_ref().unwrap().id, "w");
        (store, v, old, payload, items)
    }

    #[test]
    fn reviewed_import_applies_while_the_store_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, mut v, old, payload, items) = reviewed_import(dir.path());
        let decisions = [accept("ssh:mac", &["authMethod", "password"])];
        let staged = {
            let env = LocalEnv { vault: &v, config_dir: dir.path() };
            stage_reviewed(&store.snapshot(), &payload, &items, &decisions, &env).unwrap().unwrap()
        };
        execute(&mut store, &mut v, &dir.path().join("keys"), staged).unwrap();
        let profiles = store.profiles();
        assert_eq!(profiles[0].auth_method, AuthMethod::Agent);
        assert_eq!(
            v.get_secret(profiles[0].secret_id.as_deref().unwrap()).unwrap().as_slice(),
            b"new"
        );
        assert!(!v.has_secret(&old));
        assert_eq!(profiles[1], profile("d", "db", "h1"));
    }

    #[test]
    fn import_is_refused_when_its_match_changed_after_the_review() {
        let delete: fn(&mut Store) = |s| s.delete_profile("w").unwrap();
        let edit: fn(&mut Store) = |s| {
            let mut web = s.profiles()[0].clone();
            web.auth_method = AuthMethod::Key;
            s.upsert_profile(web).unwrap();
        };
        for change in [delete, edit] {
            let dir = tempfile::tempdir().unwrap();
            let (mut store, v, old, payload, items) = reviewed_import(dir.path());
            change(&mut store);
            let on_disk = |name: &str| std::fs::read(dir.path().join(name)).unwrap();
            let before = (store.snapshot(), on_disk("profiles.json"), on_disk("vault.enc"));

            let env = LocalEnv { vault: &v, config_dir: dir.path() };
            let decisions = [accept("ssh:mac", &["authMethod", "password"])];
            let result = stage_reviewed(&store.snapshot(), &payload, &items, &decisions, &env);
            let Err(AppError::Import(message)) = result else { panic!("not refused") };
            assert_eq!(
                message,
                "your profiles changed since this file was opened; close this tab and import \
                 the file again"
            );
            assert_eq!((store.snapshot(), on_disk("profiles.json"), on_disk("vault.enc")), before);
            assert_eq!(v.get_secret(&old).unwrap().as_slice(), b"old");
        }
    }

    #[test]
    fn replaced_password_is_deleted_after_the_commit() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, mut v, old, staged) = password_replacement(dir.path());
        execute(&mut store, &mut v, &dir.path().join("keys"), staged).unwrap();
        let p = &store.profiles()[0];
        assert_eq!(v.get_secret(p.secret_id.as_deref().unwrap()).unwrap().as_slice(), b"new");
        assert!(!v.has_secret(&old));
    }

    #[test]
    fn failed_commit_keeps_the_replaced_password() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, mut v, old, staged) = password_replacement(dir.path());
        // A directory where the temp file should be makes the store write fail.
        std::fs::create_dir(dir.path().join("profiles.json.tmp")).unwrap();
        assert!(execute(&mut store, &mut v, &dir.path().join("keys"), staged).is_err());
        assert_eq!(store.profiles()[0].secret_id.as_deref(), Some(old.as_str()));
        assert_eq!(v.get_secret(&old).unwrap().as_slice(), b"old");
    }
}
