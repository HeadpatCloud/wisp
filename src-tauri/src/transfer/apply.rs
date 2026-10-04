use std::collections::{HashMap, HashSet};
use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use same_file::is_same_file;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::bundle::{KeyFile, Payload};
use super::plan::{digest, field, group_depth, plan, same_tunnel, Plan};
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
            ItemKind::Vnc => self.data.vnc_profiles.iter().any(|p| p.id == id),
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
            let same =
                local_digests.iter().find(|(d, _)| d.is_some() && *d == content).map(|(_, l)| *l);
            let path = match (embedded, same) {
                // The same key is already here, so the profile keeps following the user's own
                // file instead of an app-managed copy.
                (Some(_), Some(l)) => l.path.clone(),
                (Some(f), None) => {
                    self.key_files.push(f);
                    format!("{KEY_PREFIX}{}", self.key_files.len() - 1)
                }
                (None, _) => k.path.clone(),
            };
            let kept = same.and_then(|l| l.secret_id.clone());
            let incoming = k.secret_id.as_deref().and_then(|id| self.payload.secrets.get(id));
            let current = kept.as_deref().and_then(|id| self.env.secret(id));
            let secret_id = match (incoming, current) {
                (Some(new), Some(old)) if old.as_slice() == new.as_bytes() => kept,
                _ => self.secret(k.secret_id.as_deref()).or(kept),
            };
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
                if fields.contains(field::NAME) {
                    next.name = g.name.clone();
                }
                if fields.contains(field::PARENT_ID) {
                    match parent {
                        Ok(p) => next.parent_id = p,
                        Err(m) => s.problem(&key, m),
                    }
                }
                if fields.contains(field::ICON) {
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
                        field::NAME => next.name = inc.name.clone(),
                        field::HOST => next.host = inc.host.clone(),
                        field::PORT => next.port = inc.port,
                        field::USERNAME => next.username = inc.username.clone(),
                        field::AUTH_METHOD => next.auth_method = inc.auth_method,
                        field::GROUP_ID => match s.group(inc.group_id.as_deref()) {
                            Ok(g) => next.group_id = g,
                            Err(m) => s.problem(&key, m),
                        },
                        field::JUMP_HOST_ID => match s.jump(inc.jump_host_id.as_deref()) {
                            Ok(j) => next.jump_host_id = j,
                            Err(m) => s.problem(&key, m),
                        },
                        field::ICON => next.icon = inc.icon.clone(),
                        field::APPEARANCE => next.appearance = inc.appearance.clone(),
                        field::TUNNELS => next.tunnels = fresh_tunnels(&next.tunnels, &inc.tunnels),
                        field::KEYS => next.keys = s.keys(&next.keys.clone(), &inc.keys),
                        field::PASSWORD => {
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
                        field::NAME => next.name = inc.name.clone(),
                        field::HOST => next.host = inc.host.clone(),
                        field::PORT => next.port = inc.port,
                        field::USERNAME => next.username = inc.username.clone(),
                        field::AUTH_METHOD => next.auth_method = inc.auth_method,
                        field::ICON => next.icon = inc.icon.clone(),
                        field::KEYS => next.keys = s.keys(&next.keys.clone(), &inc.keys),
                        field::PASSWORD => {
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
                        field::NAME => next.name = inc.name.clone(),
                        field::ENDPOINT => next.endpoint = inc.endpoint.clone(),
                        field::PORT => next.port = inc.port,
                        field::REGION => next.region = inc.region.clone(),
                        field::USE_TLS => next.use_tls = inc.use_tls,
                        field::PATH_STYLE => next.path_style = inc.path_style,
                        field::ACCESS_KEY_ID => next.access_key_id = inc.access_key_id.clone(),
                        field::BUCKET => next.bucket = inc.bucket.clone(),
                        field::ICON => next.icon = inc.icon.clone(),
                        field::PASSWORD => {
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

    for inc in &payload.vnc_profiles {
        let key = ItemKind::Vnc.key(&inc.id);
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
                    .vnc_profiles
                    .iter()
                    .find(|x| x.id == local_id)
                    .cloned()
                    .expect("matched vnc profile exists");
                for f in &fields {
                    match *f {
                        field::NAME => next.name = inc.name.clone(),
                        field::HOST => next.host = inc.host.clone(),
                        field::PORT => next.port = inc.port,
                        field::USERNAME => next.username = inc.username.clone(),
                        field::ICON => next.icon = inc.icon.clone(),
                        field::PASSWORD => {
                            s.replaced.extend(next.secret_id.take());
                            next.secret_id = s.secret(inc.secret_id.as_deref());
                        }
                        _ => {}
                    }
                }
                if let Some(slot) = s.data.vnc_profiles.iter_mut().find(|x| x.id == local_id) {
                    *slot = next;
                }
                s.summary.updated += 1;
                s.touched.push((key, local_id));
            }
            None => {
                let id =
                    s.final_id(ItemKind::Vnc, &inc.id).expect("accepted vnc profile has an id");
                let secret_id = s.secret(inc.secret_id.as_deref());
                let order = s.data.vnc_profiles.iter().map(|x| x.order + 1).max().unwrap_or(0);
                let icon = s.icon(&inc.icon);
                s.data.vnc_profiles.push(crate::store::model::VncProfile {
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
    let head: String = name.chars().take(64).collect();
    // Windows drops trailing dots and spaces from the name it creates; the stored path must be
    // the name the file really has.
    let cleaned: String = head
        .trim_end_matches(['.', ' '])
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    if cleaned.is_empty() {
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

#[cfg(unix)]
fn create_private_dir(path: &Path) -> AppResult<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?;
    Ok(())
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> AppResult<()> {
    std::fs::create_dir_all(path)?;
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
    for p in data.vnc_profiles.iter_mut() {
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
            create_private_dir(keys_dir)?;
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

// Removes the app-made key files that a changed profile used before (`previous`) and nothing
// uses now. Never a sweep: a store that loaded empty must not cost the user every imported key.
pub fn remove_unreferenced_keys(keys_dir: &Path, previous: &[ProfileKey], data: &ProfileStore) {
    // Through a linked folder the app would be deleting files somewhere else.
    if !std::fs::symlink_metadata(keys_dir).is_ok_and(|m| m.is_dir()) {
        return;
    }
    let used: Vec<&Path> = data
        .profiles
        .iter()
        .flat_map(|p| &p.keys)
        .chain(data.sftp_profiles.iter().flat_map(|p| &p.keys))
        .map(|k| Path::new(&k.path))
        .collect();
    for key in previous {
        let path = Path::new(&key.path);
        let Some(name) = path.file_name().filter(|_| path.parent() == Some(keys_dir)) else {
            continue;
        };
        // Only the `<uuid>-<name>` files `execute` writes are the app's to delete. The name must
        // be exactly one it writes: with a trailing dot, space or stream suffix Windows would
        // open a key that is still in use under its plain name.
        let app_made = name.to_str().is_some_and(|n| {
            n.get(..36).is_some_and(|id| Uuid::parse_str(id).is_ok())
                && n[36..]
                    .strip_prefix('-')
                    .is_some_and(|rest| safe_name(rest) == rest && !rest.ends_with('.'))
        });
        // Case is ignored: on Windows and macOS a differently cased path is the same file.
        // A stored path can also reach the file under another name (a trailing dot, a stream
        // suffix, a link), so the files themselves are compared too. A path that cannot be
        // opened is not this file. Network paths and anything but a regular file are not even
        // tried: tidying up must not reach out to a share (on Windows that sends the user's
        // credentials to it) or wait on a pipe.
        if app_made
            && !used.iter().any(|u| u.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name)))
            && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
            && !used.iter().any(|u| {
                u.to_str().is_some_and(|s| !super::is_network_path(s))
                    && std::fs::metadata(u).is_ok_and(|m| m.is_file())
                    && matches!(is_same_file(u, path), Ok(true))
            })
        {
            // Best-effort: a leftover key file must never fail the save or delete that got here.
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::{
        AuthMethod, Group, IconRef, Profile, ProfileKey, S3Profile, SftpProfile, VncProfile,
    };
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

    fn sftp(id: &str, name: &str, host: &str) -> SftpProfile {
        SftpProfile {
            id: id.into(),
            name: name.into(),
            host: host.into(),
            port: 22,
            username: "me".into(),
            auth_method: AuthMethod::Password,
            keys: vec![],
            secret_id: None,
            icon: IconRef::default(),
            order: 0,
        }
    }

    fn s3(id: &str, name: &str) -> S3Profile {
        S3Profile {
            id: id.into(),
            name: name.into(),
            endpoint: "s3.example.com".into(),
            port: None,
            region: "us-east-1".into(),
            use_tls: true,
            path_style: false,
            access_key_id: "AK".into(),
            secret_id: None,
            bucket: None,
            icon: IconRef::default(),
            order: 0,
        }
    }

    fn vnc(id: &str, name: &str, host: &str) -> VncProfile {
        VncProfile {
            id: id.into(),
            name: name.into(),
            host: host.into(),
            port: 5900,
            username: None,
            secret_id: None,
            icon: IconRef::default(),
            order: 0,
        }
    }

    fn store_of(profiles: Vec<Profile>) -> ProfileStore {
        ProfileStore {
            version: 1,
            groups: vec![],
            profiles,
            sftp_profiles: vec![],
            s3_profiles: vec![],
            vnc_profiles: vec![],
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
    fn embedded_key_identical_to_a_local_one_keeps_the_local_key() {
        let mut pc = profile("pc", "web", "h1");
        pc.auth_method = AuthMethod::Key;
        pc.keys = vec![
            ProfileKey { path: "C:\\keys\\a".into(), secret_id: Some("pp-a".into()) },
            ProfileKey { path: "C:\\keys\\b".into(), secret_id: None },
        ];
        let mut mac = pc.clone();
        mac.id = "mac".into();
        mac.keys = vec![
            ProfileKey { path: "/Users/me/a".into(), secret_id: Some("mac-pp".into()) },
            ProfileKey { path: "/Users/me/c".into(), secret_id: None },
        ];
        let mut payload = Payload { profiles: vec![mac], ..Default::default() };
        for (path, content) in [("/Users/me/a", b"A"), ("/Users/me/c", b"C")] {
            let data = Zeroizing::new(STANDARD.encode(content));
            payload.key_files.insert(path.into(), KeyFile { file_name: "id".into(), data });
        }
        payload.secrets.insert("mac-pp".into(), Zeroizing::new("pass".into()));
        let mut env = MapEnv::default();
        env.files.insert("C:\\keys\\a".into(), b"A".to_vec());
        env.files.insert("C:\\keys\\b".into(), b"B".to_vec());
        env.secrets.insert("pp-a".into(), b"pass".to_vec());
        let local = store_of(vec![pc.clone()]);
        let decisions = [accept("ssh:mac", &["keys"])];

        let staged =
            stage(&local, &payload, &plan(&local, &payload, &env), &decisions, &env).unwrap();
        let keys = &staged.data.profiles[0].keys;
        assert_eq!(keys[0], pc.keys[0]);
        assert_eq!(keys[1].path, format!("{KEY_PREFIX}0"));
        assert_eq!(staged.key_files.len(), 1);
        assert_eq!(STANDARD.decode(staged.key_files[0].data.as_bytes()).unwrap(), b"C");
        assert!(staged.secrets.is_empty());
        assert!(staged.replaced_secrets.is_empty());

        // A different passphrase in the export still replaces the stored one.
        payload.secrets.insert("mac-pp".into(), Zeroizing::new("other".into()));
        let staged =
            stage(&local, &payload, &plan(&local, &payload, &env), &decisions, &env).unwrap();
        let keys = &staged.data.profiles[0].keys;
        assert_eq!(keys[0].path, "C:\\keys\\a");
        assert_eq!(keys[0].secret_id, Some(format!("{SECRET_PREFIX}0")));
        assert_eq!(staged.replaced_secrets, ["pp-a"]);
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

    #[test]
    fn matched_sftp_profile_takes_only_the_accepted_fields() {
        let mut pc = sftp("pc", "files", "h1");
        pc.secret_id = Some("old".into());
        let mut mac = sftp("mac", "files (mac)", "h1");
        mac.auth_method = AuthMethod::Agent;
        mac.secret_id = Some("mac-pw".into());
        let mut payload = Payload { sftp_profiles: vec![mac], ..Default::default() };
        payload.secrets.insert("mac-pw".into(), Zeroizing::new("new".into()));
        let local = ProfileStore { sftp_profiles: vec![pc], ..store_of(vec![]) };
        let staged =
            run(&local, &payload, &[accept("sftp:mac", &["authMethod", "password"])]).unwrap();
        let p = &staged.data.sftp_profiles[0];
        assert_eq!(
            (p.id.as_str(), p.name.as_str(), p.auth_method),
            ("pc", "files", AuthMethod::Agent)
        );
        assert_eq!(p.secret_id, Some(format!("{SECRET_PREFIX}0")));
        assert_eq!(staged.secrets[0].as_str(), "new");
        assert_eq!(staged.replaced_secrets, ["old"]);
        assert_eq!(staged.summary, ApplySummary { added: 0, updated: 1 });
    }

    #[test]
    fn keys_row_carries_over_or_replaces_the_local_passphrases() {
        let mut pc = profile("pc", "web", "h1");
        pc.auth_method = AuthMethod::Key;
        pc.keys = vec![
            ProfileKey { path: "C:\\keys\\a".into(), secret_id: Some("pp-a".into()) },
            ProfileKey { path: "C:\\keys\\b".into(), secret_id: Some("pp-b".into()) },
        ];
        let mut mac = pc.clone();
        mac.id = "mac".into();
        mac.keys = vec![
            ProfileKey { path: "/Users/me/a".into(), secret_id: Some("mac-pp-a".into()) },
            ProfileKey { path: "/Users/me/c".into(), secret_id: Some("mac-pp-c".into()) },
        ];
        let mut payload = Payload { profiles: vec![mac], ..Default::default() };
        for (path, content) in [("/Users/me/a", b"A"), ("/Users/me/c", b"C")] {
            let data = Zeroizing::new(STANDARD.encode(content));
            payload.key_files.insert(path.into(), KeyFile { file_name: "id".into(), data });
        }
        // The export carries no passphrase for the first key.
        payload.secrets.insert("mac-pp-c".into(), Zeroizing::new("pass-c".into()));
        let mut env = MapEnv::default();
        env.files.insert("C:\\keys\\a".into(), b"A".to_vec());
        env.files.insert("C:\\keys\\b".into(), b"B".to_vec());
        env.secrets.insert("pp-a".into(), b"pass-a".to_vec());
        env.secrets.insert("pp-b".into(), b"pass-b".to_vec());
        let local = store_of(vec![pc]);

        let planned = plan(&local, &payload, &env);
        let staged =
            stage(&local, &payload, &planned, &[accept("ssh:mac", &["keys"])], &env).unwrap();
        let keys = &staged.data.profiles[0].keys;
        assert_eq!(keys[0].secret_id.as_deref(), Some("pp-a"));
        assert_eq!(keys[1].secret_id, Some(format!("{SECRET_PREFIX}0")));
        assert_eq!(staged.secrets[0].as_str(), "pass-c");
        assert_eq!(staged.replaced_secrets, ["pp-b"]);
    }

    #[test]
    fn jump_host_reference_lands_on_the_matched_local_profile() {
        let local = store_of(vec![profile("b-pc", "bastion", "hb"), profile("w-pc", "web", "h1")]);
        let mut web = profile("w-mac", "web", "h1");
        web.jump_host_id = Some("b-mac".into());
        let bastion = profile("b-mac", "bastion", "hb");
        let payload = Payload { profiles: vec![web, bastion], ..Default::default() };
        let staged = run(&local, &payload, &[accept("ssh:w-mac", &["jumpHostId"])]).unwrap();
        assert_eq!(staged.data.profiles.len(), 2);
        assert_eq!(staged.data.profiles[1].jump_host_id.as_deref(), Some("b-pc"));
    }

    #[test]
    fn jump_host_reference_follows_a_jump_host_added_as_new() {
        let local = store_of(vec![profile("b-pc", "bastion", "hb")]);
        let mut web = profile("w-mac", "web", "h1");
        web.jump_host_id = Some("b-mac".into());
        let mut bastion = profile("b-mac", "bastion", "hb");
        bastion.auth_method = AuthMethod::Agent;
        let payload = Payload { profiles: vec![web, bastion], ..Default::default() };
        let as_new =
            ItemDecision { key: "ssh:b-mac".into(), accept: true, as_new: true, fields: vec![] };
        let staged = run(&local, &payload, &[accept("ssh:w-mac", &[]), as_new]).unwrap();
        let profiles = &staged.data.profiles;
        assert_eq!(profiles.len(), 3);
        let added = profiles.iter().find(|p| p.auth_method == AuthMethod::Agent).unwrap();
        assert!(added.id != "b-pc" && added.id != "b-mac");
        let web = profiles.iter().find(|p| p.name == "web").unwrap();
        assert_eq!(web.jump_host_id.as_ref(), Some(&added.id));
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
            let mode = std::fs::metadata(dir.path().join("keys")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn new_sftp_profile_is_written_with_its_embedded_key_and_secrets() {
        let mut inc = sftp("f", "files", "h9");
        inc.auth_method = AuthMethod::Key;
        inc.secret_id = Some("mac-pw".into());
        inc.keys =
            vec![ProfileKey { path: "/Users/me/id".into(), secret_id: Some("mac-pp".into()) }];
        let mut payload = Payload { sftp_profiles: vec![inc], ..Default::default() };
        payload.secrets.insert("mac-pw".into(), Zeroizing::new("pw".into()));
        payload.secrets.insert("mac-pp".into(), Zeroizing::new("pp".into()));
        payload.key_files.insert(
            "/Users/me/id".into(),
            KeyFile { file_name: "id".into(), data: Zeroizing::new(STANDARD.encode(b"K")) },
        );
        let local =
            ProfileStore { sftp_profiles: vec![sftp("x", "other", "h1")], ..store_of(vec![]) };
        let staged = run(&local, &payload, &[accept("sftp:f", &[])]).unwrap();
        assert_eq!(staged.summary, ApplySummary { added: 1, updated: 0 });

        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        let mut v = vault(dir.path());
        execute(&mut store, &mut v, &dir.path().join("keys"), staged).unwrap();
        let p = &store.sftp_profiles()[1];
        assert_eq!((p.id.as_str(), p.name.as_str(), p.order), ("f", "files", 1));
        assert_eq!(p.keys.len(), 1);
        assert_eq!(std::fs::read(&p.keys[0].path).unwrap(), b"K");
        let secret = |id: &Option<String>| v.get_secret(id.as_deref().unwrap()).unwrap().to_vec();
        assert_eq!(secret(&p.keys[0].secret_id), b"pp");
        assert_eq!(secret(&p.secret_id), b"pw");
    }

    #[test]
    fn new_s3_profile_is_written_with_its_secret() {
        let mut inc = s3("s", "backups");
        inc.secret_id = Some("mac-sk".into());
        let mut payload = Payload { s3_profiles: vec![inc.clone()], ..Default::default() };
        payload.secrets.insert("mac-sk".into(), Zeroizing::new("sk".into()));
        let staged = run(&store_of(vec![]), &payload, &[accept("s3:s", &[])]).unwrap();
        assert_eq!(staged.summary, ApplySummary { added: 1, updated: 0 });

        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        let mut v = vault(dir.path());
        execute(&mut store, &mut v, &dir.path().join("keys"), staged).unwrap();
        let p = &store.s3_profiles()[0];
        assert_eq!(v.get_secret(p.secret_id.as_deref().unwrap()).unwrap().as_slice(), b"sk");
        assert_eq!(S3Profile { secret_id: inc.secret_id.clone(), ..p.clone() }, inc);
    }

    // This bundle carries no VNC profiles, so the import has to leave them as they are.
    #[test]
    fn import_leaves_vnc_profiles_as_they_were() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        let mut v = vault(dir.path());
        let desk = VncProfile {
            id: "v1".into(),
            name: "desk".into(),
            host: "10.0.0.5".into(),
            port: 5901,
            username: Some("faye".into()),
            secret_id: Some(v.set_secret(b"pw").unwrap()),
            icon: IconRef::Custom { path: "icons/6f1c2a9e-8d0b-4c57-9a3e-2b7d5e41f0c8.png".into() },
            order: 3,
        };
        store.upsert_vnc_profile(desk.clone()).unwrap();

        let payload = Payload { profiles: vec![profile("n", "new", "h9")], ..Default::default() };
        let staged = run(&store.snapshot(), &payload, &[accept("ssh:n", &[])]).unwrap();
        let summary = execute(&mut store, &mut v, &dir.path().join("keys"), staged).unwrap();
        assert_eq!(summary, ApplySummary { added: 1, updated: 0 });
        assert_eq!(store.profiles().len(), 1);
        assert_eq!(store.vnc_profiles(), [desk.clone()]);
        assert_eq!(Store::load(dir.path().to_path_buf()).unwrap().vnc_profiles(), [desk.clone()]);
        assert_eq!(v.get_secret(desk.secret_id.as_deref().unwrap()).unwrap().as_slice(), b"pw");
    }

    #[test]
    fn new_vnc_profile_is_written_with_its_secret() {
        let inc = VncProfile {
            username: Some("faye".into()),
            secret_id: Some("mac-pw".into()),
            ..vnc("v", "desk", "h9")
        };
        let mut payload = Payload { vnc_profiles: vec![inc.clone()], ..Default::default() };
        payload.secrets.insert("mac-pw".into(), Zeroizing::new("pw".into()));
        let local = ProfileStore {
            vnc_profiles: vec![VncProfile { order: 3, ..vnc("x", "other", "h1") }],
            ..store_of(vec![])
        };
        let staged = run(&local, &payload, &[accept("vnc:v", &[])]).unwrap();
        assert_eq!(staged.summary, ApplySummary { added: 1, updated: 0 });
        assert_eq!(staged.data.vnc_profiles[1].secret_id, Some(format!("{SECRET_PREFIX}0")));

        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        let mut v = vault(dir.path());
        execute(&mut store, &mut v, &dir.path().join("keys"), staged).unwrap();
        assert_eq!(store.vnc_profiles()[0], local.vnc_profiles[0]);
        let p = &store.vnc_profiles()[1];
        assert_eq!(v.get_secret(p.secret_id.as_deref().unwrap()).unwrap().as_slice(), b"pw");
        assert_eq!(p.order, 4);
        assert_eq!(VncProfile { secret_id: inc.secret_id.clone(), order: 0, ..p.clone() }, inc);
        let reloaded = Store::load(dir.path().to_path_buf()).unwrap();
        assert_eq!(reloaded.vnc_profiles(), store.vnc_profiles());
    }

    #[test]
    fn new_vnc_profile_drops_a_password_and_icon_the_bundle_does_not_bring() {
        let inc = VncProfile {
            secret_id: Some("mac-pw".into()),
            icon: IconRef::Custom { path: "icons/6f1c2a9e-8d0b-4c57-9a3e-2b7d5e41f0c8.png".into() },
            ..vnc("v", "desk", "h9")
        };
        let payload = Payload { vnc_profiles: vec![inc], ..Default::default() };
        let staged = run(&store_of(vec![]), &payload, &[accept("vnc:v", &[])]).unwrap();
        let p = &staged.data.vnc_profiles[0];
        assert_eq!((p.id.as_str(), &p.secret_id, &p.icon), ("v", &None, &IconRef::default()));
        assert!(staged.secrets.is_empty());
    }

    #[test]
    fn matched_vnc_profile_takes_only_the_accepted_fields() {
        let pc = VncProfile { secret_id: Some("old".into()), ..vnc("pc", "desk", "h1") };
        let mac = VncProfile {
            name: "desk (mac)".into(),
            host: "h2".into(),
            port: 5901,
            username: Some("faye".into()),
            secret_id: Some("mac-pw".into()),
            icon: IconRef::Builtin { name: "monitor".into() },
            ..pc.clone()
        };
        let mut payload = Payload { vnc_profiles: vec![mac.clone()], ..Default::default() };
        payload.secrets.insert("mac-pw".into(), Zeroizing::new("new".into()));
        let local = ProfileStore { vnc_profiles: vec![pc.clone()], ..store_of(vec![]) };

        let staged = run(&local, &payload, &[accept("vnc:pc", &["port", "username"])]).unwrap();
        let some = VncProfile { port: 5901, username: Some("faye".into()), ..pc };
        assert_eq!(staged.data.vnc_profiles, [some]);
        assert!(staged.secrets.is_empty() && staged.replaced_secrets.is_empty());
        assert_eq!(staged.summary, ApplySummary { added: 0, updated: 1 });

        let every = ["name", "host", "port", "username", "icon", "password"];
        let staged = run(&local, &payload, &[accept("vnc:pc", &every)]).unwrap();
        let all = VncProfile { secret_id: Some(format!("{SECRET_PREFIX}0")), ..mac };
        assert_eq!(staged.data.vnc_profiles, [all]);
        assert_eq!(staged.secrets[0].as_str(), "new");
        assert_eq!(staged.replaced_secrets, ["old"]);
        assert_eq!(staged.summary, ApplySummary { added: 0, updated: 1 });
    }

    #[test]
    fn declined_vnc_profiles_leave_everything_untouched() {
        let pc = VncProfile { secret_id: Some("old".into()), ..vnc("pc", "desk", "h1") };
        let mac = VncProfile { port: 5901, secret_id: Some("mac-pw".into()), ..pc.clone() };
        let mut payload =
            Payload { vnc_profiles: vec![mac, vnc("n", "new", "h9")], ..Default::default() };
        payload.secrets.insert("mac-pw".into(), Zeroizing::new("new".into()));
        let local = ProfileStore { vnc_profiles: vec![pc], ..store_of(vec![]) };
        let decline = |key: &str| ItemDecision {
            key: key.into(),
            accept: false,
            as_new: false,
            fields: vec!["port".into(), "password".into()],
        };
        for decisions in [
            vec![],
            vec![decline("vnc:pc"), decline("vnc:n")],
            vec![accept("vnc:pc", &[])],
        ] {
            let staged = run(&local, &payload, &decisions).unwrap();
            assert_eq!(staged.data, local);
            assert!(staged.secrets.is_empty() && staged.replaced_secrets.is_empty());
            assert_eq!(staged.summary, ApplySummary { added: 0, updated: 0 });
        }
    }

    #[test]
    fn vnc_profile_added_as_new_gets_its_own_id() {
        let pc = vnc("pc", "desk", "h1");
        let local = ProfileStore { vnc_profiles: vec![pc.clone()], ..store_of(vec![]) };
        let inc = VncProfile { port: 5901, ..pc.clone() };
        let payload = Payload { vnc_profiles: vec![inc], ..Default::default() };
        let d = ItemDecision { key: "vnc:pc".into(), accept: true, as_new: true, fields: vec![] };
        let staged = run(&local, &payload, &[d]).unwrap();
        assert_eq!(staged.data.vnc_profiles.len(), 2);
        assert_eq!(staged.data.vnc_profiles[0], pc);
        let added = &staged.data.vnc_profiles[1];
        assert_ne!(added.id, "pc");
        assert_eq!((added.port, added.order), (5901, 1));
        assert_eq!(staged.summary, ApplySummary { added: 1, updated: 0 });
    }

    // A stored VNC profile whose password is in the vault, and an import that replaces that
    // password and adds a second profile with a password of its own.
    fn vnc_import(dir: &std::path::Path) -> (Store, Vault, String, Staged) {
        let mut store = Store::load(dir.to_path_buf()).unwrap();
        let mut v = vault(dir);
        let old = v.set_secret(b"old").unwrap();
        let pc = VncProfile { secret_id: Some(old.clone()), ..vnc("pc", "desk", "h1") };
        store.upsert_vnc_profile(pc).unwrap();
        let mac = VncProfile { secret_id: Some("mac-pw".into()), ..vnc("mac", "desk", "h1") };
        let lab = VncProfile { secret_id: Some("lab-pw".into()), ..vnc("lab", "lab", "h2") };
        let mut payload = Payload { vnc_profiles: vec![mac, lab], ..Default::default() };
        payload.secrets.insert("mac-pw".into(), Zeroizing::new("new".into()));
        payload.secrets.insert("lab-pw".into(), Zeroizing::new("lab".into()));
        let decisions = [accept("vnc:mac", &["password"]), accept("vnc:lab", &[])];
        let staged = run(&store.snapshot(), &payload, &decisions).unwrap();
        (store, v, old, staged)
    }

    #[test]
    fn vnc_import_stores_the_new_passwords_and_deletes_the_replaced_one() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, mut v, old, staged) = vnc_import(dir.path());
        let summary = execute(&mut store, &mut v, &dir.path().join("keys"), staged).unwrap();
        assert_eq!(summary, ApplySummary { added: 1, updated: 1 });
        let profiles = store.vnc_profiles();
        let ids: Vec<_> = profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["pc", "lab"]);
        let secret =
            |p: &VncProfile| v.get_secret(p.secret_id.as_deref().unwrap()).unwrap().to_vec();
        assert_eq!(secret(&profiles[0]), b"new");
        assert_eq!(secret(&profiles[1]), b"lab");
        assert!(!v.has_secret(&old));
        assert_eq!(Store::load(dir.path().to_path_buf()).unwrap().vnc_profiles(), profiles);
    }

    #[test]
    fn failed_vnc_import_leaves_the_store_and_the_vault_as_they_were() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, mut v, old, staged) = vnc_import(dir.path());
        assert_eq!(staged.secrets.len(), 2);
        assert_eq!(staged.replaced_secrets, [old.clone()]);
        let on_disk = |name: &str| std::fs::read(dir.path().join(name)).unwrap();
        let before = (store.snapshot(), on_disk("profiles.json"), on_disk("vault.enc"));
        // A directory where the temp file should be makes the store write fail.
        std::fs::create_dir(dir.path().join("profiles.json.tmp")).unwrap();
        assert!(execute(&mut store, &mut v, &dir.path().join("keys"), staged).is_err());
        assert_eq!((store.snapshot(), on_disk("profiles.json"), on_disk("vault.enc")), before);
        assert_eq!(v.get_secret(&old).unwrap().as_slice(), b"old");
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

    // A file named the way `execute` names an imported key.
    fn app_key(folder: &Path, n: u8) -> ProfileKey {
        let path = folder.join(format!("6f1c2a9e-8d0b-4c57-9a3e-2b7d5e41f0c{n}-id"));
        std::fs::write(&path, b"K").unwrap();
        ProfileKey { path: path.to_string_lossy().into_owned(), secret_id: None }
    }

    fn on_disk(key: &ProfileKey) -> bool {
        Path::new(&key.path).is_file()
    }

    #[test]
    fn empty_store_without_candidates_removes_no_key_files() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        remove_unreferenced_keys(&keys, &[], &store_of(vec![]));
        assert!(!keys.exists());

        std::fs::create_dir(&keys).unwrap();
        let imported = app_key(&keys, 1);
        remove_unreferenced_keys(&keys, &[], &store_of(vec![]));
        assert!(on_disk(&imported));
    }

    #[test]
    fn deleting_a_profile_removes_only_its_own_app_made_keys() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        std::fs::create_dir_all(keys.join("sub")).unwrap();
        let own = app_key(&keys, 1);
        let shared = app_key(&keys, 2);
        let shared_with_sftp = app_key(&keys, 3);
        let other = app_key(&keys, 4);
        let never_a_candidate = app_key(&keys, 5);
        let nested = app_key(&keys.join("sub"), 6);
        let outside = app_key(dir.path(), 7);
        let hand_made = ProfileKey {
            path: keys.join("id_ed25519").to_string_lossy().into_owned(),
            secret_id: None,
        };
        std::fs::write(&hand_made.path, b"K").unwrap();

        let mut web = profile("w", "web", "h1");
        web.keys = vec![
            own.clone(),
            shared.clone(),
            shared_with_sftp.clone(),
            nested.clone(),
            outside.clone(),
            hand_made.clone(),
        ];
        let mut db = profile("d", "db", "h2");
        db.keys = vec![shared.clone(), other.clone()];
        let mut files = sftp("f", "files", "h1");
        files.keys =
            vec![ProfileKey { path: shared_with_sftp.path.to_uppercase(), secret_id: None }];
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        let data = ProfileStore { sftp_profiles: vec![files], ..store_of(vec![web, db]) };
        store.commit(data).unwrap();

        let deleted = store.profiles().into_iter().find(|p| p.id == "w").unwrap();
        store.delete_profile("w").unwrap();
        remove_unreferenced_keys(&keys, &deleted.keys, &store.snapshot());
        assert!(!on_disk(&own));
        assert!(on_disk(&shared) && on_disk(&shared_with_sftp) && on_disk(&other));
        assert!(on_disk(&never_a_candidate));
        assert!(on_disk(&nested) && on_disk(&outside) && on_disk(&hand_made));
    }

    #[test]
    fn dropping_one_of_two_keys_removes_only_that_one() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        std::fs::create_dir(&keys).unwrap();
        let (kept, dropped) = (app_key(&keys, 1), app_key(&keys, 2));
        let mut web = profile("w", "web", "h1");
        web.keys = vec![kept.clone(), dropped.clone()];
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        store.commit(store_of(vec![web.clone()])).unwrap();

        let previous = store.profiles().into_iter().find(|p| p.id == "w").unwrap();
        web.keys.truncate(1);
        store.upsert_profile(web).unwrap();
        remove_unreferenced_keys(&keys, &previous.keys, &store.snapshot());
        assert!(on_disk(&kept) && !on_disk(&dropped));
    }

    #[test]
    fn another_spelling_of_a_key_still_in_use_is_not_removed() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        std::fs::create_dir(&keys).unwrap();
        let used = app_key(&keys, 1);
        let mut web = profile("w", "web", "h1");
        web.keys = vec![used.clone()];
        let spellings: Vec<ProfileKey> = [".", " ", "::$DATA"]
            .iter()
            .map(|tail| ProfileKey { path: format!("{}{tail}", used.path), secret_id: None })
            .collect();
        remove_unreferenced_keys(&keys, &spellings, &store_of(vec![web]));
        assert!(on_disk(&used));
    }

    #[test]
    fn safe_name_drops_trailing_dots_and_spaces() {
        assert_eq!(safe_name("id."), "id");
        assert_eq!(safe_name("id. "), "id");
        assert_eq!(safe_name("..."), "key");
        assert_eq!(safe_name(&format!("{}.pub", "a".repeat(63))), "a".repeat(63));
    }

    #[cfg(unix)]
    fn link_dir(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    // A junction, because a directory symlink needs a privilege on Windows.
    #[cfg(windows)]
    fn link_dir(target: &Path, link: &Path) {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .args([link, target])
            .output()
            .unwrap();
        assert!(made.status.success());
    }

    #[test]
    fn linked_keys_folder_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let keys = dir.path().join("keys");
        link_dir(&elsewhere, &keys);
        let through_link = app_key(&keys, 1);
        remove_unreferenced_keys(&keys, &[through_link.clone()], &store_of(vec![]));
        assert!(on_disk(&through_link));

        let direct = app_key(&elsewhere, 1);
        remove_unreferenced_keys(&elsewhere, &[direct.clone()], &store_of(vec![]));
        assert!(!on_disk(&direct) && !on_disk(&through_link));
    }

    // Other paths to the file of `key`, which is in `<dir>/real/keys`; `<dir>/via` links to
    // `<dir>/real`. On Windows none of them ends in the file's own name.
    #[cfg(windows)]
    fn other_paths(dir: &Path, key: &ProfileKey) -> Vec<String> {
        let name = Path::new(&key.path).file_name().unwrap().to_string_lossy().into_owned();
        let keys = dir.join("real").join("keys").to_string_lossy().into_owned();
        vec![
            format!("{}.", key.path),
            format!("{}::$DATA", key.path),
            format!("{}\\{name}.", keys.to_ascii_uppercase()),
            format!("{keys}\\sub\\..\\{name}."),
            format!("{}\\keys\\{name}.", dir.join("via").to_string_lossy()),
        ]
    }

    // Only the symlink has another name here; the other two are the same file under its own.
    #[cfg(unix)]
    fn other_paths(dir: &Path, key: &ProfileKey) -> Vec<String> {
        let name = Path::new(&key.path).file_name().unwrap();
        let link = dir.join("id_link");
        std::os::unix::fs::symlink(&key.path, &link).unwrap();
        [link, dir.join("real/keys/sub/..").join(name), dir.join("via/keys").join(name)]
            .map(|path| path.to_string_lossy().into_owned())
            .into()
    }

    #[test]
    fn key_a_profile_reaches_through_another_path_is_not_removed() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("real");
        let keys = config.join("keys");
        std::fs::create_dir_all(keys.join("sub")).unwrap();
        link_dir(&config, &dir.path().join("via"));
        let used = app_key(&keys, 1);
        let unused = app_key(&keys, 2);
        let previous = [used.clone(), unused.clone()];
        for path in other_paths(dir.path(), &used) {
            assert_eq!(std::fs::read(&path).unwrap(), b"K", "{path}");
            let mut web = profile("w", "web", "h1");
            web.keys = vec![ProfileKey { path: path.clone(), secret_id: None }];
            remove_unreferenced_keys(&keys, &previous, &store_of(vec![web]));
            assert!(on_disk(&used), "{path}");
            assert!(!on_disk(&unused), "{path}");
        }
    }

    #[test]
    fn stored_path_to_a_missing_file_does_not_keep_another_key() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        std::fs::create_dir(&keys).unwrap();
        let dropped = app_key(&keys, 1);
        let mut web = profile("w", "web", "h1");
        let gone = keys.join("gone").to_string_lossy().into_owned();
        web.keys = vec![ProfileKey { path: gone, secret_id: None }];
        remove_unreferenced_keys(&keys, &[dropped.clone()], &store_of(vec![web]));
        assert!(!on_disk(&dropped));
    }

    #[test]
    fn stored_path_to_a_directory_does_not_keep_another_key() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        std::fs::create_dir(&keys).unwrap();
        let dropped = app_key(&keys, 1);
        let mut web = profile("w", "web", "h1");
        let folder = dir.path().to_string_lossy().into_owned();
        web.keys = vec![ProfileKey { path: folder, secret_id: None }];
        remove_unreferenced_keys(&keys, &[dropped.clone()], &store_of(vec![web]));
        assert!(!on_disk(&dropped));
    }

    #[cfg(windows)]
    #[test]
    fn stored_network_path_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        std::fs::create_dir(&keys).unwrap();
        let dropped = app_key(&keys, 1);
        let mut web = profile("w", "web", "h1");
        // An address reserved for documentation: nothing answers there, and Windows needs about
        // 20 seconds to find that out.
        let share = "\\\\192.0.2.1\\share\\id".to_string();
        web.keys = vec![ProfileKey { path: share, secret_id: None }];
        let started = std::time::Instant::now();
        remove_unreferenced_keys(&keys, &[dropped.clone()], &store_of(vec![web]));
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "{:?}", started.elapsed());
        assert!(!on_disk(&dropped));
    }

    #[cfg(unix)]
    #[test]
    fn stored_path_to_a_fifo_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        std::fs::create_dir(&keys).unwrap();
        let fifo = dir.path().join("fifo");
        match std::process::Command::new("mkfifo").arg(&fifo).status() {
            Ok(status) if status.success() => {}
            failed => {
                eprintln!("skipped: mkfifo could not create a FIFO ({failed:?})");
                return;
            }
        }
        let dropped = app_key(&keys, 1);
        let previous = [dropped.clone()];
        let mut web = profile("w", "web", "h1");
        web.keys = vec![ProfileKey { path: fifo.to_string_lossy().into_owned(), secret_id: None }];
        let data = store_of(vec![web]);
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            remove_unreferenced_keys(&keys, &previous, &data);
            done.send(()).unwrap();
        });
        // Opening a FIFO for reading waits for a writer that never comes.
        let waited = finished.recv_timeout(std::time::Duration::from_secs(5));
        assert!(waited.is_ok(), "blocked on the FIFO");
        assert!(!on_disk(&dropped));
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
