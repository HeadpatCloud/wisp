use std::collections::{HashMap, HashSet};
use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use same_file::is_same_file;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::bundle::{KeyFile, PayloadV3};
use super::plan::{field, folder_depth, gateway_secret, own_secret, plan, same_tunnel, Plan};
use super::{ApplySummary, Env, ItemDecision, ItemKind, ItemProblem, ReviewItem};
use crate::error::{AppError, AppResult};
use crate::store::hosts::{
    Connection, ConnectionKind, Folder, Host, HostsFile, Login, OwnLogin, OwnSftp, RdpGatewayConn,
    SavedLogin, SavedLoginRef, SshKey, Tag, THIS_COMPUTER,
};
use crate::store::model::{AuthMethod, IconRef, Tunnel};
use crate::store::Store;
use crate::vault::Vault;

pub const SECRET_PREFIX: &str = "pending-secret:";
pub const KEY_PREFIX: &str = "pending-key:";

pub struct Staged {
    pub data: HostsFile,
    pub secrets: Vec<Zeroizing<String>>,
    pub key_files: Vec<KeyFile>,
    pub summary: ApplySummary,
}

type Ids = HashMap<String, Option<String>>;

struct Stager<'a> {
    payload: &'a PayloadV3,
    plan: &'a Plan,
    env: &'a dyn Env,
    decisions: HashMap<&'a str, &'a ItemDecision>,
    data: HostsFile,
    secrets: Vec<Zeroizing<String>>,
    key_files: Vec<KeyFile>,
    problems: Vec<ItemProblem>,
    summary: ApplySummary,
    folder_ids: Ids,
    tag_ids: Ids,
    key_ids: Ids,
    login_ids: Ids,
    conn_ids: Ids,
    touched: Vec<(String, String)>,
}

fn fresh() -> String {
    Uuid::new_v4().to_string()
}

fn fresh_tunnels(local: &[Tunnel], incoming: &[Tunnel]) -> Vec<Tunnel> {
    incoming
        .iter()
        .map(|t| Tunnel {
            id: local.iter().find(|l| same_tunnel(l, t)).map(|l| l.id.clone()).unwrap_or_else(fresh),
            ..t.clone()
        })
        .collect()
}

fn set_own_secret(login: &mut Login, secret: Option<String>) -> bool {
    match login {
        Login::Own(o) => {
            o.secret_id = secret;
            true
        }
        Login::Saved(_) => false,
    }
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

    // Final local id for an incoming item, or None when it won't exist after apply. An incoming
    // id that already exists locally is never reused for a separate item.
    fn final_id(&self, key: &str, id: &str, taken: bool) -> Option<String> {
        let (accept, as_new, _) = self.decision(key);
        match self.plan.matches.get(key) {
            Some(local) if !(accept && as_new) => Some(local.clone()),
            _ if accept => Some(if as_new || taken { fresh() } else { id.to_string() }),
            _ => None,
        }
    }

    // Where a match was taken as a separate copy, or nothing matched: the item is new.
    fn is_new(&self, key: &str) -> bool {
        let (accept, as_new, _) = self.decision(key);
        !self.plan.matches.contains_key(key) || (accept && as_new)
    }

    fn problem(&mut self, key: &str, message: String) {
        self.problems.push(ItemProblem { key: key.into(), message });
    }

    fn resolve(map: &Ids, id: &str, here: bool, gone: impl FnOnce() -> String) -> Result<String, String> {
        match map.get(id) {
            Some(Some(final_id)) => Ok(final_id.clone()),
            Some(None) => Err(gone()),
            None if here => Ok(id.to_string()),
            None => Err(gone()),
        }
    }

    fn folder(&self, id: Option<&str>) -> Result<Option<String>, String> {
        let Some(id) = id else { return Ok(None) };
        let here = self.data.folders.iter().any(|f| f.id == id);
        if !here && !self.folder_ids.contains_key(id) {
            return Ok(None);
        }
        Self::resolve(&self.folder_ids, id, here, || {
            let name = self.payload.folders.iter().find(|f| f.id == id).map(|f| f.name.as_str()).unwrap_or(id);
            format!("Its folder \"{name}\" isn't being imported")
        })
        .map(Some)
    }

    fn tags(&self, ids: &[String]) -> Result<Vec<String>, String> {
        ids.iter()
            .map(|id| {
                let here = self.data.tags.iter().any(|t| &t.id == id);
                Self::resolve(&self.tag_ids, id, here, || {
                    let name = self.payload.tags.iter().find(|t| &t.id == id).map(|t| t.name.as_str()).unwrap_or(id);
                    format!("Its tag \"{name}\" isn't being imported")
                })
            })
            .collect()
    }

    fn keys(&self, ids: &[String]) -> Result<Vec<String>, String> {
        ids.iter()
            .map(|id| {
                let here = self.data.keys.iter().any(|k| &k.id == id);
                Self::resolve(&self.key_ids, id, here, || {
                    let name = self.payload.keys.iter().find(|k| &k.id == id).map(|k| k.path.as_str()).unwrap_or(id);
                    format!("Its key \"{name}\" isn't being imported")
                })
            })
            .collect()
    }

    fn jump(&self, id: Option<&str>) -> Result<Option<String>, String> {
        let Some(id) = id else { return Ok(None) };
        let here = self.data.connections().any(|c| c.id == id);
        // Neither exported nor here: the review told the user it connects directly.
        if !here && !self.conn_ids.contains_key(id) {
            return Ok(None);
        }
        Self::resolve(&self.conn_ids, id, here, || "Its jump host isn't being imported".into()).map(Some)
    }

    fn secret(&mut self, incoming: Option<&str>) -> Option<String> {
        let value = incoming.and_then(|id| self.payload.secrets.get(id))?;
        self.secrets.push(value.clone());
        Some(format!("{SECRET_PREFIX}{}", self.secrets.len() - 1))
    }

    // The incoming login in local terms. `secret` is what an own login's password becomes.
    fn login(&self, inc: &Login, secret: Option<String>) -> Result<Login, String> {
        match inc {
            Login::Saved(s) => {
                let here = self.data.logins.iter().any(|l| l.id == s.login_id);
                let id = Self::resolve(&self.login_ids, &s.login_id, here, || {
                    let name = self
                        .payload
                        .logins
                        .iter()
                        .find(|l| l.id == s.login_id)
                        .map(|l| l.name.as_str())
                        .unwrap_or(&s.login_id);
                    format!("Its saved login \"{name}\" isn't being imported")
                })?;
                Ok(Login::Saved(SavedLoginRef { login_id: id, extra: Default::default() }))
            }
            Login::Own(o) => Ok(Login::Own(OwnLogin {
                username: o.username.clone(),
                auth_method: o.auth_method,
                key_ids: self.keys(&o.key_ids)?,
                secret_id: secret,
                extra: Default::default(),
            })),
        }
    }

    fn icon(&self, icon: &IconRef) -> IconRef {
        match icon {
            IconRef::Custom { path } if !self.env.icon_exists(path) => IconRef::default(),
            other => other.clone(),
        }
    }

    fn new_connection(&mut self, inc: &Connection, id: String) -> Result<Connection, String> {
        let kind = match &inc.kind {
            ConnectionKind::Ssh(s) => {
                let secret = self.secret(own_secret(&s.login));
                let mut next = s.clone();
                next.login = self.login(&s.login, secret)?;
                next.jump_host_id = self.jump(s.jump_host_id.as_deref())?;
                next.tunnels = fresh_tunnels(&[], &s.tunnels);
                next.extra = Default::default();
                ConnectionKind::Ssh(next)
            }
            ConnectionKind::Sftp(s) => {
                let mut next = s.clone();
                next.own = match &s.own {
                    None => None,
                    Some(o) => {
                        let secret = self.secret(own_secret(&o.login));
                        Some(OwnSftp { port: o.port, login: self.login(&o.login, secret)?, extra: Default::default() })
                    }
                };
                next.extra = Default::default();
                ConnectionKind::Sftp(next)
            }
            ConnectionKind::Ftp(f) => {
                let secret = self.secret(own_secret(&f.login));
                let mut next = f.clone();
                next.login = self.login(&f.login, secret)?;
                next.extra = Default::default();
                ConnectionKind::Ftp(next)
            }
            ConnectionKind::S3(s) => {
                let mut next = s.clone();
                next.secret_id = self.secret(s.secret_id.as_deref());
                next.extra = Default::default();
                ConnectionKind::S3(next)
            }
            ConnectionKind::Rdp(r) => {
                let secret = self.secret(own_secret(&r.login));
                let mut next = r.clone();
                next.login = self.login(&r.login, secret)?;
                next.shared_folder = r.shared_folder.clone().filter(|f| self.env.dir_exists(&f.path));
                next.gateway = match &r.gateway {
                    None => None,
                    Some(g) => {
                        let login = match &g.login {
                            None => None,
                            Some(l) => {
                                let secret = self.secret(own_secret(l));
                                Some(self.login(l, secret)?)
                            }
                        };
                        Some(RdpGatewayConn { host: g.host.clone(), port: g.port, login, extra: Default::default() })
                    }
                };
                next.extra = Default::default();
                ConnectionKind::Rdp(next)
            }
            ConnectionKind::Vnc(v) => {
                let mut next = v.clone();
                next.secret_id = self.secret(v.secret_id.as_deref());
                next.extra = Default::default();
                ConnectionKind::Vnc(next)
            }
            ConnectionKind::Local(_) => return Err("Local shells can't be imported".into()),
        };
        Ok(Connection { id, label: inc.label.clone(), address: inc.address.clone(), kind, extra: Default::default() })
    }

    // Applies the accepted rows of a matched connection, in a fixed order: a login before its
    // password, a gateway before its password.
    fn update_connection(&mut self, key: &str, next: &mut Connection, inc: &Connection, fields: &HashSet<&str>) {
        let has = |f: &str| fields.contains(f);
        if has(field::LABEL) {
            next.label = inc.label.clone();
        }
        if has(field::ADDRESS) {
            next.address = inc.address.clone();
        }
        let mut failed = vec![];
        match (&mut next.kind, &inc.kind) {
            (ConnectionKind::Ssh(n), ConnectionKind::Ssh(i)) => {
                if has(field::PORT) {
                    n.port = i.port;
                }
                if has(field::LOGIN) {
                    let kept = own_secret(&n.login).map(str::to_string);
                    match self.login(&i.login, kept) {
                        Ok(l) => n.login = l,
                        Err(m) => failed.push(m),
                    }
                }
                if has(field::PASSWORD) {
                    let secret = self.secret(own_secret(&i.login));
                    if !set_own_secret(&mut n.login, secret) {
                        failed.push("The password needs a login of its own".into());
                    }
                }
                if has(field::JUMP_HOST_ID) {
                    match self.jump(i.jump_host_id.as_deref()) {
                        Ok(j) => n.jump_host_id = j,
                        Err(m) => failed.push(m),
                    }
                }
                if has(field::TUNNELS) {
                    n.tunnels = fresh_tunnels(&n.tunnels, &i.tunnels);
                }
                if has(field::APPEARANCE) {
                    n.appearance = i.appearance.clone();
                }
            }
            (ConnectionKind::Sftp(n), ConnectionKind::Sftp(i)) => {
                if has(field::SFTP_LOGIN) {
                    let kept = n.own.as_ref().and_then(|o| own_secret(&o.login)).map(str::to_string);
                    n.own = match &i.own {
                        None => None,
                        Some(o) => match self.login(&o.login, kept) {
                            Ok(login) => Some(OwnSftp { port: o.port, login, extra: Default::default() }),
                            Err(m) => {
                                failed.push(m);
                                n.own.clone()
                            }
                        },
                    };
                }
                if has(field::PASSWORD) {
                    let secret = self.secret(i.own.as_ref().and_then(|o| own_secret(&o.login)));
                    if !n.own.as_mut().is_some_and(|o| set_own_secret(&mut o.login, secret)) {
                        failed.push("The password needs a login of its own".into());
                    }
                }
                if has(field::START_FOLDER) {
                    n.start_folder = i.start_folder.clone();
                }
            }
            (ConnectionKind::Ftp(n), ConnectionKind::Ftp(i)) => {
                if has(field::PORT) {
                    n.port = i.port;
                }
                if has(field::LOGIN) {
                    let kept = own_secret(&n.login).map(str::to_string);
                    match self.login(&i.login, kept) {
                        Ok(l) => n.login = l,
                        Err(m) => failed.push(m),
                    }
                }
                if has(field::PASSWORD) {
                    let secret = self.secret(own_secret(&i.login));
                    if !set_own_secret(&mut n.login, secret) {
                        failed.push("The password needs a login of its own".into());
                    }
                }
                if has(field::SECURE) {
                    n.secure = i.secure;
                }
                if has(field::ALLOW_INVALID_CERT) {
                    n.allow_invalid_cert = i.allow_invalid_cert;
                }
                if has(field::IGNORE_HOSTNAME) {
                    n.ignore_hostname = i.ignore_hostname;
                }
            }
            (ConnectionKind::S3(n), ConnectionKind::S3(i)) => {
                if has(field::PORT) {
                    n.port = i.port;
                }
                if has(field::REGION) {
                    n.region = i.region.clone();
                }
                if has(field::USE_TLS) {
                    n.use_tls = i.use_tls;
                }
                if has(field::PATH_STYLE) {
                    n.path_style = i.path_style;
                }
                if has(field::ACCESS_KEY_ID) {
                    n.access_key_id = i.access_key_id.clone();
                }
                if has(field::BUCKET) {
                    n.bucket = i.bucket.clone();
                }
                if has(field::PASSWORD) {
                    n.secret_id = self.secret(i.secret_id.as_deref());
                }
            }
            (ConnectionKind::Rdp(n), ConnectionKind::Rdp(i)) => {
                if has(field::PORT) {
                    n.port = i.port;
                }
                if has(field::LOGIN) {
                    let kept = own_secret(&n.login).map(str::to_string);
                    match self.login(&i.login, kept) {
                        Ok(l) => n.login = l,
                        Err(m) => failed.push(m),
                    }
                }
                if has(field::PASSWORD) {
                    let secret = self.secret(own_secret(&i.login));
                    if !set_own_secret(&mut n.login, secret) {
                        failed.push("The password needs a login of its own".into());
                    }
                }
                if has(field::DOMAIN) {
                    n.domain = i.domain.clone();
                }
                if has(field::KEYBOARD_LAYOUT) {
                    n.keyboard_layout = i.keyboard_layout;
                }
                if has(field::AUDIO) {
                    n.audio = i.audio;
                }
                if has(field::SHARED_FOLDER) {
                    n.shared_folder = i.shared_folder.clone();
                }
                if has(field::GATEWAY) {
                    let kept = gateway_secret(&n.gateway).map(str::to_string);
                    n.gateway = match &i.gateway {
                        None => None,
                        Some(g) => {
                            let login = match &g.login {
                                None => Ok(None),
                                Some(l) => self.login(l, kept).map(Some),
                            };
                            match login {
                                Ok(login) => Some(RdpGatewayConn {
                                    host: g.host.clone(),
                                    port: g.port,
                                    login,
                                    extra: Default::default(),
                                }),
                                Err(m) => {
                                    failed.push(m);
                                    n.gateway.clone()
                                }
                            }
                        }
                    };
                }
                if has(field::GATEWAY_PASSWORD) {
                    let secret = self.secret(gateway_secret(&i.gateway));
                    let login = n.gateway.as_mut().and_then(|g| g.login.as_mut());
                    if !login.is_some_and(|l| set_own_secret(l, secret)) {
                        failed.push("Gateway password needs the incoming RD Gateway".into());
                    }
                }
            }
            (ConnectionKind::Vnc(n), ConnectionKind::Vnc(i)) => {
                if has(field::PORT) {
                    n.port = i.port;
                }
                if has(field::USERNAME) {
                    n.username = i.username.clone();
                }
                if has(field::PASSWORD) {
                    n.secret_id = self.secret(i.secret_id.as_deref());
                }
            }
            _ => {}
        }
        for m in failed {
            self.problem(key, m);
        }
    }

    fn order_in(&self, folder: &Option<String>) -> u32 {
        self.data.hosts.iter().filter(|h| &h.folder_id == folder).map(|h| h.order + 1).max().unwrap_or(0)
    }

    fn stage_folders(&mut self) {
        let mut folders: Vec<&Folder> = self.payload.folders.iter().collect();
        folders.sort_by_key(|f| folder_depth(f, &self.payload.folders));
        for f in folders {
            let key = ItemKind::Folder.key(&f.id);
            let (accept, _, fields) = self.decision(&key);
            if !accept {
                continue;
            }
            let parent = self.folder(f.parent_id.as_deref());
            if self.is_new(&key) {
                let id = self.folder_ids.get(&f.id).cloned().flatten().expect("accepted folder has an id");
                let parent_id = parent.unwrap_or_else(|m| {
                    self.problem(&key, m);
                    None
                });
                let order = self.data.folders.iter().filter(|x| x.parent_id == parent_id).map(|x| x.order + 1).max().unwrap_or(0);
                let icon = self.icon(&f.icon);
                self.data.folders.push(Folder { id: id.clone(), name: f.name.clone(), parent_id, icon, order, extra: Default::default() });
                self.summary.added += 1;
                self.touched.push((key, id));
            } else if !fields.is_empty() {
                let local = self.plan.matches[&key].clone();
                let at = self.data.folders.iter().position(|x| x.id == local).expect("matched folder exists");
                if fields.contains(field::NAME) {
                    self.data.folders[at].name = f.name.clone();
                }
                if fields.contains(field::PARENT_ID) {
                    match parent {
                        Ok(p) => self.data.folders[at].parent_id = p,
                        Err(m) => self.problem(&key, m),
                    }
                }
                if fields.contains(field::ICON) {
                    self.data.folders[at].icon = f.icon.clone();
                }
                self.summary.updated += 1;
                self.touched.push((key, local));
            }
        }
    }

    fn stage_tags(&mut self) {
        for t in self.payload.tags.iter() {
            let key = ItemKind::Tag.key(&t.id);
            let (accept, _, fields) = self.decision(&key);
            if !accept {
                continue;
            }
            if self.is_new(&key) {
                let id = self.tag_ids.get(&t.id).cloned().flatten().expect("accepted tag has an id");
                self.data.tags.push(Tag { id, extra: Default::default(), ..t.clone() });
                self.summary.added += 1;
            } else if !fields.is_empty() {
                let local = self.plan.matches[&key].clone();
                let next = self.data.tags.iter_mut().find(|x| x.id == local).expect("matched tag exists");
                for f in &fields {
                    match *f {
                        field::NAME => next.name = t.name.clone(),
                        field::COLOR => next.color = t.color.clone(),
                        field::SHOW_COLOR => next.show_color = t.show_color,
                        field::ASK_BEFORE_CLOSE => next.ask_before_close = t.ask_before_close,
                        field::ASK_BEFORE_PASTE => next.ask_before_paste = t.ask_before_paste,
                        field::TERMINAL_COLORS => next.terminal_colors = t.terminal_colors.clone(),
                        _ => {}
                    }
                }
                self.summary.updated += 1;
            }
        }
    }

    fn key_path(&mut self, k: &SshKey) -> Result<String, String> {
        if let Some(f) = self.payload.key_files.get(&k.path).cloned() {
            self.key_files.push(f);
            return Ok(format!("{KEY_PREFIX}{}", self.key_files.len() - 1));
        }
        // A key path on a network share is never stored: opening it at connect time would send
        // the user's credentials to whatever server the bundle named. Nor is a path spelled like
        // a placeholder, which would alias another item's embedded key file.
        if super::is_network_path(&k.path) || k.path.starts_with(KEY_PREFIX) {
            return Err("Its key file is on a network share; Wisp won't open it".into());
        }
        Ok(k.path.clone())
    }

    fn stage_keys(&mut self) {
        for k in self.payload.keys.iter() {
            let key = ItemKind::Key.key(&k.id);
            let (accept, _, fields) = self.decision(&key);
            if !accept {
                continue;
            }
            if self.is_new(&key) {
                let id = self.key_ids.get(&k.id).cloned().flatten().expect("accepted key has an id");
                let path = self.key_path(k).unwrap_or_else(|m| {
                    self.problem(&key, m);
                    k.path.clone()
                });
                let secret_id = self.secret(k.secret_id.as_deref());
                self.data.keys.push(SshKey { id: id.clone(), path, secret_id, extra: Default::default() });
                self.summary.added += 1;
                self.touched.push((key, id));
            } else if !fields.is_empty() {
                let local = self.plan.matches[&key].clone();
                let at = self.data.keys.iter().position(|x| x.id == local).expect("matched key exists");
                if fields.contains(field::KEY_FILE) {
                    match self.key_path(k) {
                        Ok(path) => self.data.keys[at].path = path,
                        Err(m) => self.problem(&key, m),
                    }
                }
                if fields.contains(field::PASSPHRASE) {
                    self.data.keys[at].secret_id = self.secret(k.secret_id.as_deref());
                }
                self.summary.updated += 1;
                self.touched.push((key, local));
            }
        }
    }

    fn stage_logins(&mut self) {
        for l in self.payload.logins.iter() {
            let key = ItemKind::Login.key(&l.id);
            let (accept, _, fields) = self.decision(&key);
            if !accept {
                continue;
            }
            if self.is_new(&key) {
                let id = self.login_ids.get(&l.id).cloned().flatten().expect("accepted login has an id");
                let key_ids = self.keys(&l.key_ids).unwrap_or_else(|m| {
                    self.problem(&key, m);
                    vec![]
                });
                let secret_id = self.secret(l.secret_id.as_deref());
                self.data.logins.push(SavedLogin { id: id.clone(), key_ids, secret_id, extra: Default::default(), ..l.clone() });
                self.summary.added += 1;
                self.touched.push((key, id));
            } else if !fields.is_empty() {
                let local = self.plan.matches[&key].clone();
                let at = self.data.logins.iter().position(|x| x.id == local).expect("matched login exists");
                if fields.contains(field::NAME) {
                    self.data.logins[at].name = l.name.clone();
                }
                if fields.contains(field::USERNAME) {
                    self.data.logins[at].username = l.username.clone();
                }
                if fields.contains(field::AUTH_METHOD) {
                    self.data.logins[at].auth_method = l.auth_method;
                }
                if fields.contains(field::KEYS) {
                    match self.keys(&l.key_ids) {
                        Ok(ids) => self.data.logins[at].key_ids = ids,
                        Err(m) => self.problem(&key, m),
                    }
                }
                if fields.contains(field::PASSWORD) {
                    self.data.logins[at].secret_id = self.secret(l.secret_id.as_deref());
                }
                self.summary.updated += 1;
                self.touched.push((key, local));
            }
        }
    }

    fn stage_hosts(&mut self) {
        let hosts: Vec<&Host> = self.payload.hosts.iter().filter(|h| h.id != THIS_COMPUTER).collect();
        for h in hosts {
            let key = ItemKind::Host.key(&h.id);
            let (accept, _, fields) = self.decision(&key);
            if self.is_new(&key) {
                if !accept {
                    continue;
                }
                let taken = self.data.hosts.iter().any(|x| x.id == h.id);
                let id = self.final_id(&key, &h.id, taken).expect("accepted host has an id");
                let folder_id = self.folder(h.folder_id.as_deref()).unwrap_or_else(|m| {
                    self.problem(&key, m);
                    None
                });
                let tag_ids = self.tags(&h.tag_ids).unwrap_or_else(|m| {
                    self.problem(&key, m);
                    vec![]
                });
                let mut connections = vec![];
                for c in &h.connections {
                    let Some(Some(cid)) = self.conn_ids.get(&c.id).cloned() else { continue };
                    let ckey = ItemKind::Connection.key(&c.id);
                    match self.new_connection(c, cid.clone()) {
                        Ok(conn) => connections.push(conn),
                        Err(m) => self.problem(&ckey, m),
                    }
                    self.touched.push((ckey, cid));
                }
                let order = self.order_in(&folder_id);
                let icon = self.icon(&h.icon);
                self.data.hosts.push(Host {
                    id: id.clone(),
                    name: h.name.clone(),
                    address: h.address.clone(),
                    folder_id,
                    tag_ids,
                    icon,
                    color: h.color.clone(),
                    order,
                    connections,
                    extra: Default::default(),
                });
                self.summary.added += 1;
                self.touched.push((key, id));
                continue;
            }

            let local = self.plan.matches[&key].clone();
            let at = self.data.hosts.iter().position(|x| x.id == local).expect("matched host exists");
            if accept && !fields.is_empty() {
                if fields.contains(field::NAME) {
                    self.data.hosts[at].name = h.name.clone();
                }
                if fields.contains(field::ADDRESS) {
                    self.data.hosts[at].address = h.address.clone();
                }
                if fields.contains(field::FOLDER_ID) {
                    match self.folder(h.folder_id.as_deref()) {
                        Ok(f) => self.data.hosts[at].folder_id = f,
                        Err(m) => self.problem(&key, m),
                    }
                }
                if fields.contains(field::TAGS) {
                    match self.tags(&h.tag_ids) {
                        Ok(t) => self.data.hosts[at].tag_ids = t,
                        Err(m) => self.problem(&key, m),
                    }
                }
                if fields.contains(field::ICON) {
                    self.data.hosts[at].icon = h.icon.clone();
                }
                if fields.contains(field::COLOR) {
                    self.data.hosts[at].color = h.color.clone();
                }
                self.summary.updated += 1;
                self.touched.push((key.clone(), local.clone()));
            }
            for c in &h.connections {
                let ckey = ItemKind::Connection.key(&c.id);
                let (caccept, _, cfields) = self.decision(&ckey);
                if !caccept {
                    continue;
                }
                let Some(Some(cid)) = self.conn_ids.get(&c.id).cloned() else { continue };
                if self.is_new(&ckey) {
                    match self.new_connection(c, cid.clone()) {
                        Ok(conn) => {
                            let at = self.data.hosts.iter().position(|x| x.id == local).expect("matched host exists");
                            self.data.hosts[at].connections.push(conn);
                        }
                        Err(m) => self.problem(&ckey, m),
                    }
                    self.summary.added += 1;
                } else if !cfields.is_empty() {
                    let at = self.data.hosts.iter().position(|x| x.id == local).expect("matched host exists");
                    let ci = self.data.hosts[at].connections.iter().position(|x| x.id == cid).expect("matched connection exists");
                    let mut next = self.data.hosts[at].connections[ci].clone();
                    self.update_connection(&ckey, &mut next, c, &cfields);
                    self.data.hosts[at].connections[ci] = next;
                    self.summary.updated += 1;
                }
                self.touched.push((ckey, cid));
            }
        }
    }

    fn check(&mut self) {
        let mut found: Vec<(String, String)> = vec![];
        for (key, id) in &self.touched {
            let key_login_without_keys = |l: &Login| {
                matches!(l, Login::Own(o) if o.auth_method == AuthMethod::Key && o.key_ids.is_empty())
            };
            if let Some(l) = self.data.logins.iter().find(|l| &l.id == id) {
                if l.auth_method == AuthMethod::Key && l.key_ids.is_empty() {
                    found.push((key.clone(), "Key login needs at least one private key".into()));
                }
            }
            if let Some(c) = self.data.connections().find(|c| &c.id == id) {
                if c.kind.logins().into_iter().any(key_login_without_keys) {
                    found.push((key.clone(), "Key login needs at least one private key".into()));
                }
                let jump = |id: &str| {
                    self.data.connections().find(|c| c.id == id).and_then(|c| match &c.kind {
                        ConnectionKind::Ssh(s) => s.jump_host_id.clone(),
                        _ => None,
                    })
                };
                let mut seen = HashSet::new();
                let mut hop = jump(id);
                while let Some(h) = hop {
                    if &h == id || !seen.insert(h.clone()) {
                        found.push((key.clone(), "Its jump hosts would form a loop".into()));
                        break;
                    }
                    hop = jump(&h);
                }
            }
            if let Some(f) = self.data.folders.iter().find(|f| &f.id == id) {
                let mut seen = HashSet::new();
                let mut parent = f.parent_id.clone();
                while let Some(p) = parent {
                    if &p == id || !seen.insert(p.clone()) {
                        found.push((key.clone(), "Its parent folders would form a loop".into()));
                        break;
                    }
                    parent = self.data.folders.iter().find(|f| f.id == p).and_then(|f| f.parent_id.clone());
                }
            }
        }
        for (key, message) in found {
            self.problem(&key, message);
        }
        if self.problems.is_empty() {
            if let Err(AppError::Invalid(m)) = self.data.validate() {
                self.problem("", m);
            }
        }
    }
}

pub fn stage(
    local: &HostsFile,
    payload: &PayloadV3,
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
        problems: vec![],
        summary: ApplySummary { added: 0, updated: 0 },
        folder_ids: HashMap::new(),
        tag_ids: HashMap::new(),
        key_ids: HashMap::new(),
        login_ids: HashMap::new(),
        conn_ids: HashMap::new(),
        touched: vec![],
    };
    for f in &payload.folders {
        let taken = local.folders.iter().any(|x| x.id == f.id);
        let id = s.final_id(&ItemKind::Folder.key(&f.id), &f.id, taken);
        s.folder_ids.insert(f.id.clone(), id);
    }
    for t in &payload.tags {
        let taken = local.tags.iter().any(|x| x.id == t.id);
        let id = s.final_id(&ItemKind::Tag.key(&t.id), &t.id, taken);
        s.tag_ids.insert(t.id.clone(), id);
    }
    for k in &payload.keys {
        let taken = local.keys.iter().any(|x| x.id == k.id);
        let id = s.final_id(&ItemKind::Key.key(&k.id), &k.id, taken);
        s.key_ids.insert(k.id.clone(), id);
    }
    for l in &payload.logins {
        let taken = local.logins.iter().any(|x| x.id == l.id);
        let id = s.final_id(&ItemKind::Login.key(&l.id), &l.id, taken);
        s.login_ids.insert(l.id.clone(), id);
    }
    for h in payload.hosts.iter().filter(|h| h.id != THIS_COMPUTER) {
        let hkey = ItemKind::Host.key(&h.id);
        let (host_accept, host_as_new, _) = s.decision(&hkey);
        let host_matched = plan.matches.contains_key(&hkey);
        for c in &h.connections {
            let ckey = ItemKind::Connection.key(&c.id);
            let taken = local.connections().any(|x| x.id == c.id);
            let id = if host_matched && host_as_new && host_accept {
                Some(fresh())
            } else if host_matched {
                s.final_id(&ckey, &c.id, taken)
            } else {
                let (accept, _, _) = s.decision(&ckey);
                (host_accept && accept).then(|| if taken { fresh() } else { c.id.clone() })
            };
            s.conn_ids.insert(c.id.clone(), id);
        }
    }

    s.stage_folders();
    s.stage_tags();
    s.stage_keys();
    s.stage_logins();
    s.stage_hosts();
    s.check();

    if !s.problems.is_empty() {
        return Err(s.problems);
    }
    Ok(Staged { data: s.data, secrets: s.secrets, key_files: s.key_files, summary: s.summary })
}

pub fn stage_reviewed(
    local: &HostsFile,
    payload: &PayloadV3,
    reviewed: &[ReviewItem],
    decisions: &[ItemDecision],
    env: &dyn Env,
) -> AppResult<Result<Staged, Vec<ItemProblem>>> {
    let planned = plan(local, payload, env);
    // The decisions were made on the stored review; on a different plan they could land on a
    // host the user never saw.
    if planned.items != reviewed {
        return Err(AppError::Import(
            "your hosts changed since this file was opened; import the file again".into(),
        ));
    }
    Ok(stage(local, payload, &planned, decisions, env))
}

pub(crate) fn safe_name(name: &str) -> String {
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
    value.strip_prefix(prefix).and_then(|i| i.parse::<usize>().ok()).and_then(|i| real.get(i).cloned())
}

fn for_each_secret(file: &mut HostsFile, mut f: impl FnMut(&mut Option<String>)) {
    let own = |login: &mut Login, f: &mut dyn FnMut(&mut Option<String>)| {
        if let Login::Own(o) = login {
            f(&mut o.secret_id);
        }
    };
    for l in file.logins.iter_mut() {
        f(&mut l.secret_id);
    }
    for k in file.keys.iter_mut() {
        f(&mut k.secret_id);
    }
    for c in file.hosts.iter_mut().flat_map(|h| h.connections.iter_mut()) {
        match &mut c.kind {
            ConnectionKind::Ssh(s) => own(&mut s.login, &mut f),
            ConnectionKind::Sftp(s) => {
                if let Some(o) = s.own.as_mut() {
                    own(&mut o.login, &mut f);
                }
            }
            ConnectionKind::Ftp(x) => own(&mut x.login, &mut f),
            ConnectionKind::S3(x) => f(&mut x.secret_id),
            ConnectionKind::Rdp(r) => {
                own(&mut r.login, &mut f);
                if let Some(l) = r.gateway.as_mut().and_then(|g| g.login.as_mut()) {
                    own(l, &mut f);
                }
            }
            ConnectionKind::Vnc(v) => f(&mut v.secret_id),
            ConnectionKind::Local(_) => {}
        }
    }
}

fn substitute(file: &mut HostsFile, secrets: &[String], keys: &[String]) {
    for_each_secret(file, |id| {
        if let Some(real) = id.as_deref().and_then(|v| placeholder(v, SECRET_PREFIX, secrets)) {
            *id = Some(real);
        }
    });
    for k in file.keys.iter_mut() {
        if let Some(real) = placeholder(&k.path, KEY_PREFIX, keys) {
            k.path = real;
        }
    }
}

// Everything is written or nothing: on any error the key files and vault entries this apply made
// are removed again and the store is never swapped. Secrets the import replaced are deleted by
// the caller once the new hosts are in (store_cmds::after_change).
pub fn execute(store: &mut Store, vault: &mut Vault, keys_dir: &Path, staged: Staged) -> AppResult<ApplySummary> {
    let Staged { mut data, secrets, key_files, summary } = staged;
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
        store.replace_hosts(data)
    })();
    if let Err(e) = result {
        for path in &written {
            let _ = std::fs::remove_file(path);
        }
        for id in &new_secrets {
            let _ = vault.delete_secret(id);
        }
        return Err(e);
    }
    Ok(summary)
}

// Removes the app-made key files that a change dropped (`previous`) and nothing uses now (`used`).
// Never a sweep: a store that loaded empty must not cost the user every imported key.
pub fn remove_unreferenced_key_files(keys_dir: &Path, previous: &[String], used: &[String]) {
    // Through a linked folder the app would be deleting files somewhere else.
    if !std::fs::symlink_metadata(keys_dir).is_ok_and(|m| m.is_dir()) {
        return;
    }
    let used: Vec<&Path> = used.iter().map(Path::new).collect();
    for key in previous {
        let path = Path::new(key);
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
    use crate::store::hosts::fixtures::{file, host, own, ssh, vnc};
    use crate::store::hosts::SshConn;
    use crate::transfer::MapEnv;

    fn vault(dir: &Path) -> Vault {
        Vault::open_with_key(dir.join("vault.enc"), Zeroizing::new([3u8; 32])).unwrap()
    }

    fn accept_all(plan: &Plan) -> Vec<ItemDecision> {
        plan.items
            .iter()
            .map(|i| ItemDecision {
                key: i.key.clone(),
                accept: true,
                as_new: false,
                fields: i.fields.iter().map(|f| f.field.clone()).collect(),
            })
            .collect()
    }

    fn run(local: &HostsFile, payload: &PayloadV3, decide: impl Fn(&Plan) -> Vec<ItemDecision>) -> Result<Staged, Vec<ItemProblem>> {
        let env = MapEnv::default();
        let planned = plan(local, payload, &env);
        stage(local, payload, &planned, &decide(&planned), &env)
    }

    fn jumping(id: &str, jump: &str) -> Connection {
        let mut c = ssh(id, 22, own("u", None));
        if let ConnectionKind::Ssh(SshConn { jump_host_id, .. }) = &mut c.kind {
            *jump_host_id = Some(jump.into());
        }
        c
    }

    fn jump_of(file: &HostsFile, host_name: &str) -> Option<String> {
        let h = file.hosts.iter().find(|h| h.name == host_name).unwrap();
        match &h.connections[0].kind {
            ConnectionKind::Ssh(s) => s.jump_host_id.clone(),
            _ => None,
        }
    }

    #[test]
    fn accepting_a_new_host_adds_it_with_its_connections() {
        let payload = PayloadV3 {
            hosts: vec![host("h", "nas", "nas.local", vec![ssh("c1", 22, own("admin", None)), vnc("c2", None)])],
            ..Default::default()
        };
        let staged = run(&file(vec![]), &payload, accept_all).unwrap();
        let added = staged.data.hosts.iter().find(|h| h.name == "nas").unwrap();
        let ids: Vec<_> = added.connections.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["c1", "c2"]);
        assert_eq!(staged.summary.added, 1);
    }

    #[test]
    fn declining_a_new_host_skips_its_connections() {
        let payload = PayloadV3 {
            hosts: vec![host("h", "nas", "nas.local", vec![vnc("c1", None)])],
            ..Default::default()
        };
        let staged = run(&file(vec![]), &payload, |p| {
            p.items.iter().map(|i| ItemDecision { key: i.key.clone(), accept: i.key != "host:h", as_new: false, fields: vec![] }).collect()
        })
        .unwrap();
        assert!(staged.data.hosts.is_empty());
    }

    #[test]
    fn a_new_connection_lands_on_the_matched_host() {
        let local = file(vec![host("l", "nas", "nas.local", vec![vnc("lv", None)])]);
        let payload = PayloadV3 {
            hosts: vec![host("i", "nas", "nas.local", vec![vnc("iv", None), ssh("is", 22, own("admin", None))])],
            ..Default::default()
        };
        let staged = run(&local, &payload, accept_all).unwrap();
        assert_eq!(staged.data.hosts.len(), 1);
        let ids: Vec<_> = staged.data.hosts[0].connections.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["lv", "is"]);
    }

    #[test]
    fn a_taken_id_gets_a_fresh_one_and_references_follow() {
        let local = file(vec![host("x", "other", "10.9.9.9", vec![vnc("bastion-ssh", None)])]);
        let payload = PayloadV3 {
            hosts: vec![
                host("b", "bastion", "bastion.corp", vec![ssh("bastion-ssh", 22, own("u", None))]),
                host("t", "target", "10.0.0.2", vec![jumping("t-ssh", "bastion-ssh")]),
            ],
            ..Default::default()
        };
        let staged = run(&local, &payload, accept_all).unwrap();
        let bastion = staged.data.hosts.iter().find(|h| h.name == "bastion").unwrap();
        let fresh_id = bastion.connections[0].id.clone();
        assert_ne!(fresh_id, "bastion-ssh");
        assert_eq!(jump_of(&staged.data, "target"), Some(fresh_id));
    }

    #[test]
    fn a_declined_jump_host_is_a_problem() {
        let payload = PayloadV3 {
            hosts: vec![
                host("b", "bastion", "bastion.corp", vec![ssh("b-ssh", 22, own("u", None))]),
                host("t", "target", "10.0.0.2", vec![jumping("t-ssh", "b-ssh")]),
            ],
            ..Default::default()
        };
        let problems = run(&file(vec![]), &payload, |p| {
            p.items.iter().map(|i| ItemDecision { key: i.key.clone(), accept: i.key != "host:b", as_new: false, fields: vec![] }).collect()
        })
        .err()
        .unwrap();
        assert_eq!(problems[0].key, "conn:t-ssh");
        assert_eq!(problems[0].message, "Its jump host isn't being imported");
    }

    #[test]
    fn a_jump_host_missing_from_the_export_connects_directly() {
        let payload = PayloadV3 {
            hosts: vec![host("t", "target", "10.0.0.2", vec![jumping("t-ssh", "gone")])],
            ..Default::default()
        };
        let staged = run(&file(vec![]), &payload, accept_all).unwrap();
        assert_eq!(jump_of(&staged.data, "target"), None);
    }

    #[test]
    fn an_accepted_row_changes_only_that_field() {
        let mut lc = vnc("c", None);
        lc.label = Some("old".into());
        let local = file(vec![host("h", "nas", "nas.local", vec![lc])]);
        let mut ic = vnc("c", None);
        ic.label = Some("new".into());
        if let ConnectionKind::Vnc(v) = &mut ic.kind {
            v.port = 5901;
        }
        let payload = PayloadV3 { hosts: vec![host("h", "nas", "nas.local", vec![ic])], ..Default::default() };
        let staged = run(&local, &payload, |p| {
            p.items.iter().map(|i| ItemDecision { key: i.key.clone(), accept: true, as_new: false, fields: vec!["port".into()] }).collect()
        })
        .unwrap();
        let c = &staged.data.hosts[0].connections[0];
        assert_eq!(c.label.as_deref(), Some("old"));
        assert!(matches!(&c.kind, ConnectionKind::Vnc(v) if v.port == 5901));
    }

    #[test]
    fn a_failed_commit_leaves_vault_and_keys_as_they_were() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &[]).unwrap();
        let mut v = vault(dir.path());
        let mut payload = PayloadV3 {
            hosts: vec![host("h", "nas", "nas.local", vec![ssh("c", 22, own("admin", Some("pw")))])],
            ..Default::default()
        };
        payload.secrets.insert("pw".into(), Zeroizing::new("secret".into()));
        payload.keys.push(SshKey { id: "k".into(), path: "/remote/id".into(), secret_id: None, extra: Default::default() });
        payload.key_files.insert(
            "/remote/id".into(),
            KeyFile { file_name: "id".into(), data: Zeroizing::new(STANDARD.encode(b"KEY")) },
        );
        let staged = run(&store.hosts(), &payload, accept_all).unwrap();
        // A directory where the temp file should be makes the store write fail.
        std::fs::create_dir(dir.path().join("hosts.json.tmp")).unwrap();
        let keys = dir.path().join("keys");
        assert!(execute(&mut store, &mut v, &keys, staged).is_err());
        assert!(store.hosts().hosts.iter().all(|h| h.name != "nas"));
        assert_eq!(std::fs::read_dir(&keys).unwrap().count(), 0);
        let vault_file: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("vault.enc")).unwrap()).unwrap();
        assert_eq!(vault_file["secrets"].as_object().map(|s| s.len()).unwrap_or(0), 0);
    }

    #[test]
    fn execute_writes_secrets_and_key_files_and_points_at_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &[]).unwrap();
        let mut v = vault(dir.path());
        let key_login = Login::Own(OwnLogin {
            username: "admin".into(),
            auth_method: AuthMethod::Key,
            key_ids: vec!["k".into()],
            secret_id: None,
            extra: Default::default(),
        });
        let mut payload = PayloadV3 {
            hosts: vec![host("h", "nas", "nas.local", vec![ssh("c", 22, key_login)])],
            ..Default::default()
        };
        payload.keys.push(SshKey { id: "k".into(), path: "/remote/id".into(), secret_id: Some("pp".into()), extra: Default::default() });
        payload.secrets.insert("pp".into(), Zeroizing::new("phrase".into()));
        payload.key_files.insert(
            "/remote/id".into(),
            KeyFile { file_name: "id".into(), data: Zeroizing::new(STANDARD.encode(b"KEY")) },
        );
        let staged = run(&store.hosts(), &payload, accept_all).unwrap();
        let keys = dir.path().join("keys");
        execute(&mut store, &mut v, &keys, staged).unwrap();
        let key = store.hosts().keys.into_iter().find(|k| k.id == "k").unwrap();
        assert!(Path::new(&key.path).starts_with(&keys));
        assert_eq!(std::fs::read(&key.path).unwrap(), b"KEY");
        assert_eq!(v.get_secret(key.secret_id.as_deref().unwrap()).unwrap().as_slice(), b"phrase");
    }

    #[cfg(windows)]
    #[test]
    fn a_network_key_path_without_an_embedded_file_is_a_problem() {
        let mut payload = PayloadV3::default();
        payload.keys.push(SshKey { id: "k".into(), path: "\\\\server\\share\\id".into(), secret_id: None, extra: Default::default() });
        let problems = run(&file(vec![]), &payload, accept_all).err().unwrap();
        assert_eq!(problems[0].key, "key:k");
    }

    fn app_key(folder: &Path, n: u8) -> String {
        let path = folder.join(format!("6f1c2a9e-8d0b-4c57-9a3e-2b7d5e41f0c{n}-id"));
        std::fs::write(&path, b"K").unwrap();
        path.to_string_lossy().into_owned()
    }

    fn on_disk(path: &str) -> bool {
        Path::new(path).is_file()
    }

    #[test]
    fn nothing_dropped_removes_no_key_files() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        remove_unreferenced_key_files(&keys, &[], &[]);
        assert!(!keys.exists());

        std::fs::create_dir(&keys).unwrap();
        let imported = app_key(&keys, 1);
        remove_unreferenced_key_files(&keys, &[], &[]);
        assert!(on_disk(&imported));
    }

    #[test]
    fn only_app_made_keys_nothing_uses_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        std::fs::create_dir_all(keys.join("sub")).unwrap();
        let own = app_key(&keys, 1);
        let shared = app_key(&keys, 2);
        let shared_in_capitals = app_key(&keys, 3);
        let never_a_candidate = app_key(&keys, 5);
        let nested = app_key(&keys.join("sub"), 6);
        let outside = app_key(dir.path(), 7);
        let hand_made = keys.join("id_ed25519").to_string_lossy().into_owned();
        std::fs::write(&hand_made, b"K").unwrap();

        let previous = [own.clone(), shared.clone(), shared_in_capitals.clone(), nested.clone(), outside.clone(), hand_made.clone()];
        let used = [shared.clone(), shared_in_capitals.to_uppercase()];
        remove_unreferenced_key_files(&keys, &previous, &used);
        assert!(!on_disk(&own));
        assert!(on_disk(&shared) && on_disk(&shared_in_capitals));
        assert!(on_disk(&never_a_candidate));
        assert!(on_disk(&nested) && on_disk(&outside) && on_disk(&hand_made));
    }

    #[test]
    fn another_spelling_of_a_key_still_in_use_is_not_removed() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        std::fs::create_dir(&keys).unwrap();
        let used = app_key(&keys, 1);
        let spellings: Vec<String> = [".", " ", "::$DATA"].iter().map(|tail| format!("{used}{tail}")).collect();
        remove_unreferenced_key_files(&keys, &spellings, &[used.clone()]);
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
        remove_unreferenced_key_files(&keys, &[through_link.clone()], &[]);
        assert!(on_disk(&through_link));

        let direct = app_key(&elsewhere, 1);
        remove_unreferenced_key_files(&elsewhere, &[direct.clone()], &[]);
        assert!(!on_disk(&direct) && !on_disk(&through_link));
    }

    // Other paths to the file of `key`, which is in `<dir>/real/keys`; `<dir>/via` links to
    // `<dir>/real`. On Windows none of them ends in the file's own name.
    #[cfg(windows)]
    fn other_paths(dir: &Path, key: &str) -> Vec<String> {
        let name = Path::new(key).file_name().unwrap().to_string_lossy().into_owned();
        let keys = dir.join("real").join("keys").to_string_lossy().into_owned();
        vec![
            format!("{key}."),
            format!("{key}::$DATA"),
            format!("{}\\{name}.", keys.to_ascii_uppercase()),
            format!("{keys}\\sub\\..\\{name}."),
            format!("{}\\keys\\{name}.", dir.join("via").to_string_lossy()),
        ]
    }

    // Only the symlink has another name here; the other two are the same file under its own.
    #[cfg(unix)]
    fn other_paths(dir: &Path, key: &str) -> Vec<String> {
        let name = Path::new(key).file_name().unwrap();
        let link = dir.join("id_link");
        std::os::unix::fs::symlink(key, &link).unwrap();
        [link, dir.join("real/keys/sub/..").join(name), dir.join("via/keys").join(name)]
            .map(|path| path.to_string_lossy().into_owned())
            .into()
    }

    #[test]
    fn a_key_reached_through_another_path_is_not_removed() {
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
            remove_unreferenced_key_files(&keys, &previous, &[path.clone()]);
            assert!(on_disk(&used), "{path}");
            assert!(!on_disk(&unused), "{path}");
        }
    }

    #[test]
    fn a_used_path_to_a_missing_file_or_a_folder_keeps_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        std::fs::create_dir(&keys).unwrap();
        let dropped = app_key(&keys, 1);
        let gone = keys.join("gone").to_string_lossy().into_owned();
        remove_unreferenced_key_files(&keys, &[dropped.clone()], &[gone]);
        assert!(!on_disk(&dropped));

        let dropped = app_key(&keys, 2);
        let folder = dir.path().to_string_lossy().into_owned();
        remove_unreferenced_key_files(&keys, &[dropped.clone()], &[folder]);
        assert!(!on_disk(&dropped));
    }

    #[cfg(windows)]
    #[test]
    fn a_used_network_path_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        std::fs::create_dir(&keys).unwrap();
        let dropped = app_key(&keys, 1);
        // An address reserved for documentation: nothing answers there, and Windows needs about
        // 20 seconds to find that out.
        let share = "\\\\192.0.2.1\\share\\id".to_string();
        let started = std::time::Instant::now();
        remove_unreferenced_key_files(&keys, &[dropped.clone()], &[share]);
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "{:?}", started.elapsed());
        assert!(!on_disk(&dropped));
    }

    #[cfg(unix)]
    #[test]
    fn a_used_path_to_a_fifo_is_not_opened() {
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
        let used = [fifo.to_string_lossy().into_owned()];
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            remove_unreferenced_key_files(&keys, &previous, &used);
            done.send(()).unwrap();
        });
        // Opening a FIFO for reading waits for a writer that never comes.
        let waited = finished.recv_timeout(std::time::Duration::from_secs(5));
        assert!(waited.is_ok(), "blocked on the FIFO");
        assert!(!on_disk(&dropped));
    }
}
