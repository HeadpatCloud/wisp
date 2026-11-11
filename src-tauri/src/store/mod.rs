pub mod hosts;
pub mod io;
pub mod migrate;
pub mod model;
pub mod themes;

use std::collections::{BTreeSet, HashSet};
use std::mem;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::commands::local_cmds::ShellInfo;
use crate::error::{AppError, AppResult};
use hosts::{
    Connection, ConnectionKind, ConnectionMove, DeletedConnection, DeletedHost, DeletedItem, Folder,
    FolderMove, Host, HostMove, HostsFile, Login, MoveTarget, PendingDelete, SavedLogin, SshKey,
    Tag, THIS_COMPUTER,
};
use migrate::MigrationReport;
use model::{AuthMethod, IconRef, Profile, ProfileKey, ProfileStore, Settings};

pub struct Store {
    dir: PathBuf,
    hosts: HostsFile,
    migration: Option<MigrationReport>,
    settings: Settings,
    themes: themes::ThemesFile,
}

// Fields a later version put on a stored item are kept when the dialogs send it back without
// them. An item new to the store starts with none: a duplicate is sent as a copy of what the
// store gave, and could share a vault reference.
fn keep_kind_extras(new: &mut ConnectionKind, old: Option<&mut ConnectionKind>) {
    use ConnectionKind::*;
    let Some(old) = old else {
        match new {
            Ssh(n) => {
                n.extra.clear();
                clear_login_extras(&mut n.login);
                n.appearance.iter_mut().for_each(|a| a.extra.clear());
            }
            Sftp(n) => {
                n.extra.clear();
                n.own.iter_mut().for_each(|o| {
                    o.extra.clear();
                    clear_login_extras(&mut o.login);
                });
            }
            Ftp(n) => {
                n.extra.clear();
                clear_login_extras(&mut n.login);
            }
            S3(n) => n.extra.clear(),
            Rdp(n) => {
                n.extra.clear();
                clear_login_extras(&mut n.login);
                n.gateway.iter_mut().for_each(|g| {
                    g.extra.clear();
                    g.login.iter_mut().for_each(clear_login_extras);
                });
                n.shared_folder.iter_mut().for_each(|f| f.extra.clear());
            }
            Vnc(n) => n.extra.clear(),
            Local(n) => n.extra.clear(),
        }
        return;
    };
    match (new, old) {
        (Ssh(n), Ssh(o)) => {
            n.extra = mem::take(&mut o.extra);
            keep_login_extras(&mut n.login, &mut o.login);
            if let (Some(a), Some(kept)) = (n.appearance.as_mut(), o.appearance.as_mut()) {
                a.extra = mem::take(&mut kept.extra);
            }
        }
        (Sftp(n), Sftp(o)) => {
            n.extra = mem::take(&mut o.extra);
            if let (Some(own), Some(kept)) = (n.own.as_mut(), o.own.as_mut()) {
                own.extra = mem::take(&mut kept.extra);
                keep_login_extras(&mut own.login, &mut kept.login);
            }
        }
        (Ftp(n), Ftp(o)) => {
            n.extra = mem::take(&mut o.extra);
            keep_login_extras(&mut n.login, &mut o.login);
        }
        (S3(n), S3(o)) => n.extra = mem::take(&mut o.extra),
        (Rdp(n), Rdp(o)) => {
            n.extra = mem::take(&mut o.extra);
            keep_login_extras(&mut n.login, &mut o.login);
            if let (Some(g), Some(kept)) = (n.gateway.as_mut(), o.gateway.as_mut()) {
                g.extra = mem::take(&mut kept.extra);
                if let (Some(l), Some(kept)) = (g.login.as_mut(), kept.login.as_mut()) {
                    keep_login_extras(l, kept);
                }
            }
            if let (Some(f), Some(kept)) = (n.shared_folder.as_mut(), o.shared_folder.as_mut()) {
                f.extra = mem::take(&mut kept.extra);
            }
        }
        (Vnc(n), Vnc(o)) => n.extra = mem::take(&mut o.extra),
        (Local(n), Local(o)) => n.extra = mem::take(&mut o.extra),
        _ => {}
    }
}

fn keep_login_extras(new: &mut Login, old: &mut Login) {
    match (new, old) {
        (Login::Own(n), Login::Own(o)) => n.extra = mem::take(&mut o.extra),
        (Login::Saved(n), Login::Saved(o)) => n.extra = mem::take(&mut o.extra),
        _ => {}
    }
}

fn clear_login_extras(login: &mut Login) {
    match login {
        Login::Own(o) => o.extra.clear(),
        Login::Saved(s) => s.extra.clear(),
    }
}

// Folds a pre-multi-key profile's single key_path into the keys list. Runs on every read and
// write path, not just load, so profiles arriving from an import or the ssh-config importer
// are normalized too. The secret *moves* rather than being copied: two owners of one vault
// entry means whichever side is edited first deletes the other's passphrase.
pub(crate) fn normalize_keys(p: &mut Profile) {
    let Some(path) = p.key_path.take() else { return };
    if !p.keys.is_empty() {
        return;
    }
    let secret_id = if p.auth_method == AuthMethod::Key { p.secret_id.take() } else { None };
    p.keys.push(ProfileKey { path, secret_id });
}

fn pending(item: DeletedItem) -> PendingDelete {
    let deleted_at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    PendingDelete { id: Uuid::new_v4().to_string(), deleted_at, item, extra: Default::default() }
}

pub(crate) fn next_order(file: &HostsFile, folder_id: Option<&str>) -> u32 {
    file.hosts.iter().filter(|h| h.folder_id.as_deref() == folder_id).map(|h| h.order + 1).max().unwrap_or(0)
}

pub(crate) const ICON_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "svg"];

// A custom icon is always `icons/<uuid>.<ext>`, exactly as `import_icon` writes it. The path
// can come from an imported bundle and is joined onto the config dir, where it is read and
// later deleted. Another spelling of the same file would get past the shared-icon check.
pub(crate) fn is_icon_path(rel: &str) -> bool {
    rel.strip_prefix("icons/").and_then(|file| file.split_once('.')).is_some_and(|(id, ext)| {
        Uuid::parse_str(id).is_ok_and(|uuid| uuid.hyphenated().to_string() == id)
            && ICON_EXTENSIONS.contains(&ext)
    })
}

// The icons and key files 0.5.0's profiles.json names. Going back to 0.5.0 needs them, so no
// change in 0.6.0 deletes them. None when the file is there but can't be read: it might name
// anything, so nothing is deleted.
pub(crate) fn named_by_profiles(dir: &Path) -> Option<(BTreeSet<String>, Vec<String>)> {
    let old: ProfileStore = match io::read_json_strict(&dir.join("profiles.json")) {
        Ok(Some(old)) => old,
        Ok(None) => return Some(Default::default()),
        Err(_) => return None,
    };
    let mut icons = BTreeSet::new();
    let mut icon = |i: &IconRef| {
        if let IconRef::Custom { path } = i {
            icons.insert(path.clone());
        }
    };
    old.groups.iter().for_each(|g| icon(&g.icon));
    old.profiles.iter().for_each(|p| icon(&p.icon));
    old.sftp_profiles.iter().for_each(|p| icon(&p.icon));
    old.s3_profiles.iter().for_each(|p| icon(&p.icon));
    old.vnc_profiles.iter().for_each(|p| icon(&p.icon));
    old.rdp_profiles.iter().for_each(|p| icon(&p.icon));
    let keys = old
        .profiles
        .iter()
        .flat_map(|p| p.key_path.iter().chain(p.keys.iter().map(|k| &k.path)))
        .chain(old.sftp_profiles.iter().flat_map(|p| p.keys.iter().map(|k| &k.path)))
        .cloned()
        .collect();
    Some((icons, keys))
}

impl Store {
    // profiles.json is only ever read here: 0.5.0 keeps working from it after an update.
    pub fn load(dir: PathBuf, shells: &[ShellInfo]) -> AppResult<Self> {
        let legacy: Option<ProfileStore> =
            io::read_json_strict(&dir.join("profiles.json")).ok().flatten();

        let hosts_path = dir.join("hosts.json");
        // A corrupt file is renamed aside by read_json, and then counts as missing.
        let existing = if hosts_path.exists() {
            let file: HostsFile = io::read_json(&hosts_path)?;
            hosts_path.exists().then_some(file)
        } else {
            None
        };
        let mut migration = None;
        let mut write = existing.is_none();
        let mut hosts = match (existing, &legacy) {
            (Some(file), _) => file,
            (None, Some(old)) => {
                let (file, report) = migrate::migrate(old, shells);
                migration = (report.profiles > 0).then_some(report);
                file
            }
            (None, None) => HostsFile { version: HostsFile::CURRENT_VERSION, ..Default::default() },
        };
        if !hosts.hosts.iter().any(|h| h.id == THIS_COMPUTER) {
            hosts.hosts.insert(0, migrate::this_computer(shells));
            write = true;
        }
        if write {
            io::write_json_atomic(&hosts_path, &hosts)?;
        }

        let mut settings: Settings = io::read_json(&dir.join("settings.json"))?;
        let mut theme_file: themes::ThemesFile = io::read_json(&dir.join("themes.json"))?;
        if settings.light_theme.is_empty() {
            themes::migrate(&mut settings, &mut theme_file);
            io::write_json_atomic(&dir.join("themes.json"), &theme_file)?;
            io::write_json_atomic(&dir.join("settings.json"), &settings)?;
        }
        Ok(Self { dir, hosts, migration, settings, themes: theme_file })
    }

    pub fn hosts(&self) -> HostsFile {
        self.hosts.clone()
    }

    pub fn take_migration_report(&mut self) -> Option<MigrationReport> {
        self.migration.take()
    }

    // Written first and swapped in only on success, so a failed write leaves memory as it was.
    pub fn replace_hosts(&mut self, file: HostsFile) -> AppResult<()> {
        file.validate()?;
        io::write_json_atomic(&self.dir.join("hosts.json"), &file)?;
        self.hosts = file;
        Ok(())
    }

    // Connections the host no longer has, and that no move carries, become pending deletes; their
    // ids come back for the Undo notices.
    pub fn upsert_host(&mut self, mut host: Host, moves: Vec<ConnectionMove>) -> AppResult<Vec<String>> {
        let mut file = self.hosts.clone();
        let mut arriving: Vec<(MoveTarget, Connection)> = Vec::new();
        for m in moves {
            let mut c = m.connection;
            let stored = file.hosts.iter_mut().find_map(|h| {
                let at = h.connections.iter().position(|o| o.id == c.id)?;
                Some(h.connections.remove(at))
            });
            match stored {
                Some(mut old) => {
                    c.extra = mem::take(&mut old.extra);
                    keep_kind_extras(&mut c.kind, Some(&mut old.kind));
                }
                None => {
                    c.extra.clear();
                    keep_kind_extras(&mut c.kind, None);
                }
            }
            arriving.push((m.to, c));
        }

        let mut deleted = Vec::new();
        match file.hosts.iter_mut().find(|h| h.id == host.id) {
            Some(existing) => {
                host.extra = mem::take(&mut existing.extra);
                for c in host.connections.iter_mut() {
                    let old = existing.connections.iter_mut().find(|o| o.id == c.id);
                    match old {
                        Some(old) => {
                            c.extra = mem::take(&mut old.extra);
                            keep_kind_extras(&mut c.kind, Some(&mut old.kind));
                        }
                        None => {
                            c.extra.clear();
                            keep_kind_extras(&mut c.kind, None);
                        }
                    }
                }
                let kept: HashSet<&str> = host.connections.iter().map(|c| c.id.as_str()).collect();
                for (index, old) in existing.connections.iter().enumerate() {
                    if !kept.contains(old.id.as_str()) {
                        deleted.push(pending(DeletedItem::Connection(DeletedConnection {
                            host_id: host.id.clone(),
                            index: index as u32,
                            connection: old.clone(),
                        })));
                    }
                }
                if host.id == THIS_COMPUTER {
                    host.name = existing.name.clone();
                    host.address = existing.address.clone();
                    host.folder_id = None;
                    host.tag_ids.clear();
                }
                *existing = host;
            }
            None => {
                host.extra.clear();
                for c in host.connections.iter_mut() {
                    c.extra.clear();
                    keep_kind_extras(&mut c.kind, None);
                }
                file.hosts.push(host);
            }
        }

        for (to, c) in arriving {
            match to {
                MoveTarget::Host(t) => file
                    .hosts
                    .iter_mut()
                    .find(|h| h.id == t.host_id)
                    .ok_or_else(|| AppError::Invalid("The host to move it to no longer exists.".into()))?
                    .connections
                    .push(c),
                MoveTarget::NewHost(t) => {
                    let order = next_order(&file, t.folder_id.as_deref());
                    file.hosts.push(Host {
                        id: Uuid::new_v4().to_string(),
                        name: t.name,
                        address: t.address,
                        folder_id: t.folder_id,
                        tag_ids: vec![],
                        icon: IconRef::default(),
                        color: None,
                        order,
                        connections: vec![c],
                        extra: Default::default(),
                    });
                }
            }
        }

        let ids = deleted.iter().map(|p| p.id.clone()).collect();
        file.pending_deletes.extend(deleted);
        self.replace_hosts(file)?;
        Ok(ids)
    }

    pub fn delete_host(&mut self, id: &str) -> AppResult<String> {
        if id == THIS_COMPUTER {
            return Err(AppError::Invalid("This computer can't be deleted.".into()));
        }
        let mut file = self.hosts.clone();
        let at = file
            .hosts
            .iter()
            .position(|h| h.id == id)
            .ok_or_else(|| AppError::NotFound(format!("host {id}")))?;
        let item = pending(DeletedItem::Host(DeletedHost { host: file.hosts.remove(at) }));
        let pending_id = item.id.clone();
        file.pending_deletes.push(item);
        self.replace_hosts(file)?;
        Ok(pending_id)
    }

    pub fn undo_delete(&mut self, id: &str) -> AppResult<()> {
        let mut file = self.hosts.clone();
        let at = file
            .pending_deletes
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| AppError::Invalid("Nothing to undo.".into()))?;
        match file.pending_deletes.remove(at).item {
            DeletedItem::Host(DeletedHost { mut host }) => {
                if host.folder_id.as_ref().is_some_and(|f| !file.folders.iter().any(|x| &x.id == f)) {
                    host.folder_id = None;
                }
                host.tag_ids.retain(|t| file.tags.iter().any(|x| &x.id == t));
                file.hosts.push(host);
            }
            DeletedItem::Connection(d) => {
                let host = file
                    .hosts
                    .iter_mut()
                    .find(|h| h.id == d.host_id)
                    .ok_or_else(|| AppError::Invalid("Its host was deleted. Undo that first.".into()))?;
                let index = (d.index as usize).min(host.connections.len());
                host.connections.insert(index, d.connection);
            }
            DeletedItem::Other(_) => {
                return Err(AppError::Invalid("A newer Wisp deleted this, so only it can undo it.".into()));
            }
        }
        self.replace_hosts(file)
    }

    // None purges every pending delete, as at start.
    pub fn purge_deletes(&mut self, ids: Option<&[String]>) -> AppResult<()> {
        let mut file = self.hosts.clone();
        let count = file.pending_deletes.len();
        file.pending_deletes.retain(|p| ids.is_some_and(|ids| !ids.contains(&p.id)));
        if file.pending_deletes.len() == count {
            return Ok(());
        }
        self.replace_hosts(file)
    }

    pub fn upsert_key(&mut self, mut key: SshKey) -> AppResult<()> {
        let mut file = self.hosts.clone();
        match file.keys.iter_mut().find(|k| k.id == key.id) {
            Some(existing) => {
                key.extra = mem::take(&mut existing.extra);
                *existing = key;
            }
            None => {
                key.extra.clear();
                file.keys.push(key);
            }
        }
        self.replace_hosts(file)
    }

    pub fn upsert_login(&mut self, mut login: SavedLogin) -> AppResult<()> {
        let mut file = self.hosts.clone();
        match file.logins.iter_mut().find(|l| l.id == login.id) {
            Some(existing) => {
                login.extra = mem::take(&mut existing.extra);
                *existing = login;
            }
            None => {
                login.extra.clear();
                file.logins.push(login);
            }
        }
        self.replace_hosts(file)
    }

    pub fn delete_login(&mut self, id: &str) -> AppResult<()> {
        let mut file = self.hosts.clone();
        let Some(login) = file.logins.iter().find(|l| l.id == id) else { return Ok(()) };
        let users = file.login_users(id);
        if !users.is_empty() {
            return Err(AppError::Invalid(format!("{} is used by {}.", login.name, users.join(", "))));
        }
        file.logins.retain(|l| l.id != id);
        self.replace_hosts(file)
    }

    pub fn delete_key(&mut self, id: &str) -> AppResult<()> {
        let mut file = self.hosts.clone();
        let Some(key) = file.keys.iter().find(|k| k.id == id) else { return Ok(()) };
        let users = file.key_users(id);
        if !users.is_empty() {
            let name = std::path::Path::new(&key.path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| key.path.clone());
            return Err(AppError::Invalid(format!("{name} is used by {}.", users.join(", "))));
        }
        file.keys.retain(|k| k.id != id);
        self.replace_hosts(file)
    }

    pub fn upsert_tag(&mut self, mut tag: Tag) -> AppResult<()> {
        let mut file = self.hosts.clone();
        match file.tags.iter_mut().find(|t| t.id == tag.id) {
            Some(existing) => {
                tag.extra = mem::take(&mut existing.extra);
                *existing = tag;
            }
            None => {
                tag.extra.clear();
                file.tags.push(tag);
            }
        }
        self.replace_hosts(file)
    }

    // The tag goes from every host too, pending deletes included, so an Undo can't bring back
    // a host that names it.
    pub fn delete_tag(&mut self, id: &str) -> AppResult<()> {
        let mut file = self.hosts.clone();
        file.tags.retain(|t| t.id != id);
        let untag = |h: &mut Host| h.tag_ids.retain(|t| t != id);
        file.hosts.iter_mut().for_each(untag);
        for p in &mut file.pending_deletes {
            if let DeletedItem::Host(d) = &mut p.item {
                untag(&mut d.host);
            }
        }
        self.replace_hosts(file)
    }

    pub fn upsert_folder(&mut self, mut folder: Folder) -> AppResult<()> {
        let mut file = self.hosts.clone();
        match file.folders.iter_mut().find(|f| f.id == folder.id) {
            Some(existing) => {
                folder.extra = mem::take(&mut existing.extra);
                *existing = folder;
            }
            None => {
                folder.extra.clear();
                file.folders.push(folder);
            }
        }
        self.replace_hosts(file)
    }

    pub fn move_hosts(&mut self, moves: &[HostMove]) -> AppResult<()> {
        let mut file = self.hosts.clone();
        for m in moves {
            let host = file
                .hosts
                .iter_mut()
                .find(|h| h.id == m.id)
                .ok_or_else(|| AppError::NotFound(format!("host {}", m.id)))?;
            host.folder_id = m.folder_id.clone();
            host.order = m.order;
        }
        self.replace_hosts(file)
    }

    pub fn move_folders(&mut self, moves: &[FolderMove]) -> AppResult<()> {
        let mut file = self.hosts.clone();
        for m in moves {
            let folder = file
                .folders
                .iter_mut()
                .find(|f| f.id == m.id)
                .ok_or_else(|| AppError::NotFound(format!("folder {}", m.id)))?;
            folder.parent_id = m.parent_id.clone();
            folder.order = m.order;
        }
        self.replace_hosts(file)
    }

    fn persist_settings(&self) -> AppResult<()> {
        io::write_json_atomic(&self.dir.join("settings.json"), &self.settings)
    }

    pub fn settings(&self) -> Settings {
        self.settings.clone()
    }

    pub fn set_settings(&mut self, mut settings: Settings) -> AppResult<()> {
        settings.extra = mem::take(&mut self.settings.extra);
        self.settings = settings;
        self.persist_settings()
    }

    pub fn themes(&self) -> themes::ThemesFile {
        self.themes.clone()
    }

    pub fn set_themes(&mut self, file: themes::ThemesFile) -> AppResult<()> {
        io::write_json_atomic(&self.dir.join("themes.json"), &file)?;
        self.themes = file;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{AuthMethod, IconRef};

    #[test]
    fn icon_path_is_only_what_import_icon_writes() {
        for ext in ICON_EXTENSIONS {
            let path = format!("icons/{}.{ext}", uuid::Uuid::new_v4());
            assert!(is_icon_path(&path), "{path}");
        }

        let id = "6f1c2a9e-8d0b-4c57-9a3e-2b7d5e41f0c8";
        let others = [
            format!("icons/{}.png", id.to_uppercase()),
            format!("icons/{id}.PNG"),
            format!("icons\\{id}.png"),
            format!("icons//{id}.png"),
            format!("Icons/{id}.png"),
            format!("icons/{id}.png."),
            format!("icons/{id}.png "),
            format!("icons/{id}.png::$DATA"),
            format!("icons/{id}.bmp"),
            format!("icons/{id}"),
            format!("icons/{}.png", id.replace('-', "")),
            "icons/NUL".to_string(),
            "icons/a.png".to_string(),
        ];
        let accepted: Vec<_> = others.iter().filter(|p| is_icon_path(p)).collect();
        assert!(accepted.is_empty(), "{accepted:?}");
    }

    fn profile(id: &str) -> Profile {
        Profile {
            id: id.into(),
            name: id.into(),
            group_id: None,
            host: "h".into(),
            port: 22,
            username: "u".into(),
            auth_method: AuthMethod::Password,
            key_path: None,
            keys: vec![],
            secret_id: None,
            icon: IconRef::default(),
            order: 0,
            jump_host_id: None,
            tunnels: vec![],
            appearance: None,
            extra: Default::default(),
        }
    }

    fn shells() -> Vec<ShellInfo> {
        vec![ShellInfo { name: "Command Prompt".into(), program: "cmd.exe".into() }]
    }

    fn two_ssh_profiles(dir: &std::path::Path) -> Vec<u8> {
        let mut a = profile("a");
        a.host = "nas.local".into();
        let mut b = profile("b");
        b.host = "nas.local".into();
        b.username = "root".into();
        let old = ProfileStore {
            version: 1,
            profiles: vec![a, b],
            ..Default::default()
        };
        let bytes = serde_json::to_vec_pretty(&old).unwrap();
        std::fs::write(dir.join("profiles.json"), &bytes).unwrap();
        bytes
    }

    const SETTINGS_050: &str = r#"{
        "theme": "dark", "fontFamily": "Cascadia Mono", "fontSize": 13, "colorScheme": "default",
        "accent": "blue", "background": "slate"
    }"#;

    #[test]
    fn settings_migration_makes_tint_themes_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), SETTINGS_050).unwrap();
        let store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        let s = store.settings();
        assert_eq!((s.light_theme.as_str(), s.dark_theme.as_str()), ("paper-slate", "graphite-slate"));
        assert_eq!(s.color_scheme, "default");
        let themes = store.themes();
        let names: Vec<_> = themes.themes.iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["Paper · Slate", "Graphite · Slate"]);
        assert_eq!(themes.themes[0]["tint"], 255);
        assert_eq!(themes.themes[1]["kind"], "dark");

        drop(store);
        let again = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        assert_eq!(again.themes().themes.len(), 2);
    }

    #[test]
    fn a_teal_0_5_0_setup_gets_the_built_in_themes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            SETTINGS_050.replace("\"slate\"", "\"teal\""),
        )
        .unwrap();
        let store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        assert_eq!(store.settings().light_theme, "paper");
        assert_eq!(store.settings().dark_theme, "graphite");
        assert!(store.themes().themes.is_empty());
    }

    #[test]
    fn unknown_settings_fields_survive_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let with_future = SETTINGS_050.replace("\"accent\"", "\"fromTheFuture\": [1, 2], \"accent\"");
        std::fs::write(dir.path().join("settings.json"), with_future).unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        let mut s = store.settings();
        s.extra.clear();
        s.font_size = 15;
        store.set_settings(s).unwrap();
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("settings.json")).unwrap()).unwrap();
        assert_eq!(raw["fromTheFuture"], serde_json::json!([1, 2]));
        assert_eq!(raw["fontSize"], 15);
    }

    #[test]
    fn a_settings_file_written_by_0_6_still_has_0_5_fields() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        store.set_settings(Settings { density: "compact".into(), ..store.settings() }).unwrap();
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("settings.json")).unwrap()).unwrap();
        for field in ["theme", "fontFamily", "fontSize", "colorScheme"] {
            assert!(raw.get(field).is_some(), "{field} missing");
        }
        assert_eq!(raw["density"], "compact");
    }

    #[test]
    fn deleting_a_tag_removes_it_from_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        let tag = |id: &str| hosts::Tag {
            id: id.into(),
            name: id.to_uppercase(),
            color: "red".into(),
            show_color: true,
            ask_before_close: true,
            ask_before_paste: false,
            terminal_colors: None,
            extra: Default::default(),
        };
        store.upsert_tag(tag("prod")).unwrap();
        store.upsert_tag(tag("db")).unwrap();
        let mut web = hosts::fixtures::host("h", "web", "web.lan", vec![]);
        web.tag_ids = vec!["prod".into(), "db".into()];
        store.upsert_host(web, vec![]).unwrap();

        store.delete_tag("prod").unwrap();
        let file = store.hosts();
        assert_eq!(file.tags.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["db"]);
        assert_eq!(file.hosts.iter().find(|h| h.id == "h").unwrap().tag_ids, ["db"]);
    }

    fn saved_login(id: &str, name: &str, key_ids: Vec<String>) -> hosts::SavedLogin {
        hosts::SavedLogin {
            id: id.into(),
            name: name.into(),
            username: "deploy".into(),
            auth_method: AuthMethod::Key,
            key_ids,
            secret_id: None,
            extra: Default::default(),
        }
    }

    fn key(id: &str, path: &str) -> hosts::SshKey {
        hosts::SshKey { id: id.into(), path: path.into(), secret_id: None, extra: Default::default() }
    }

    #[test]
    fn a_login_in_use_cannot_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        store.upsert_login(saved_login("l1", "Deploy", vec![])).unwrap();
        let saved = hosts::Login::Saved(hosts::SavedLoginRef { login_id: "l1".into(), extra: Default::default() });
        let web = hosts::fixtures::host("h", "web", "web.lan", vec![hosts::fixtures::ssh("c", 22, saved)]);
        store.upsert_host(web, vec![]).unwrap();

        match store.delete_login("l1") {
            Err(AppError::Invalid(m)) => assert_eq!(m, "Deploy is used by web · SSH."),
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert_eq!(store.hosts().logins.len(), 1);

        store.delete_host("h").unwrap();
        assert!(store.delete_login("l1").is_err(), "a deleted host can still be undone");
        store.purge_deletes(None).unwrap();
        store.delete_login("l1").unwrap();
        assert!(store.hosts().logins.is_empty());
    }

    #[test]
    fn a_key_in_use_cannot_be_removed() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        store.upsert_key(key("k1", "C:/keys/id_deploy")).unwrap();
        store.upsert_key(key("k2", "C:/keys/id_spare")).unwrap();
        store.upsert_login(saved_login("l1", "Deploy", vec!["k1".into()])).unwrap();

        match store.delete_key("k1") {
            Err(AppError::Invalid(m)) => assert_eq!(m, "id_deploy is used by the login Deploy."),
            other => panic!("expected a refusal, got {other:?}"),
        }
        store.delete_key("k2").unwrap();
        assert_eq!(store.hosts().keys.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(), ["k1"]);
    }

    #[test]
    fn profiles_json_is_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let before = two_ssh_profiles(dir.path());
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        store.upsert_folder(hosts::fixtures::folder("f", None)).unwrap();
        store.upsert_host(hosts::fixtures::host("x", "x", "x", vec![]), vec![]).unwrap();
        store.move_hosts(&[HostMove { id: "x".into(), folder_id: Some("f".into()), order: 3 }]).unwrap();
        store.move_folders(&[FolderMove { id: "f".into(), parent_id: None, order: 1 }]).unwrap();
        store.replace_hosts(store.hosts()).unwrap();
        store.set_settings(Settings::default()).unwrap();
        assert_eq!(std::fs::read(dir.path().join("profiles.json")).unwrap(), before);
    }

    #[test]
    fn first_start_without_files_has_only_this_computer() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        let hosts = store.hosts().hosts;
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].id, THIS_COMPUTER);
        assert!(dir.path().join("hosts.json").is_file());
        assert_eq!(store.take_migration_report(), None);
    }

    #[test]
    fn existing_profiles_are_migrated_once_and_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let before = two_ssh_profiles(dir.path());
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        assert_eq!(
            store.take_migration_report(),
            Some(MigrationReport { profiles: 2, hosts: 1 })
        );
        assert_eq!(store.take_migration_report(), None);
        assert_eq!(std::fs::read(dir.path().join("profiles.json")).unwrap(), before);

        let mut reopened = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        assert_eq!(reopened.hosts(), store.hosts());
        assert_eq!(reopened.take_migration_report(), None);
    }

    #[test]
    fn corrupt_hosts_file_is_set_aside_and_migration_runs_again() {
        let dir = tempfile::tempdir().unwrap();
        two_ssh_profiles(dir.path());
        std::fs::write(dir.path().join("hosts.json"), "{not json").unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        assert!(dir.path().join("hosts.json.corrupt").is_file());
        assert!(store.take_migration_report().is_some());
        assert_eq!(store.hosts().hosts.len(), 2);
    }

    #[test]
    fn unreadable_profiles_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("profiles.json"), "{bad").unwrap();
        let store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        let hosts = store.hosts().hosts;
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].id, THIS_COMPUTER);
        assert_eq!(std::fs::read(dir.path().join("profiles.json")).unwrap(), b"{bad");
        assert!(!dir.path().join("profiles.json.corrupt").exists());
    }

    #[test]
    fn a_failed_write_leaves_memory_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        let before = store.hosts();
        std::fs::remove_file(dir.path().join("hosts.json")).unwrap();
        std::fs::create_dir(dir.path().join("hosts.json")).unwrap();
        let new = hosts::fixtures::host("h", "box", "10.0.0.1", vec![]);
        assert!(store.upsert_host(new, vec![]).is_err());
        assert_eq!(store.hosts(), before);
    }

    #[test]
    fn moving_a_folder_into_its_descendant_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        store.upsert_folder(hosts::fixtures::folder("a", None)).unwrap();
        store.upsert_folder(hosts::fixtures::folder("b", Some("a"))).unwrap();
        let bytes = std::fs::read(dir.path().join("hosts.json")).unwrap();
        let moved = store.move_folders(&[FolderMove {
            id: "a".into(),
            parent_id: Some("b".into()),
            order: 0,
        }]);
        assert!(matches!(moved, Err(AppError::Invalid(_))));
        assert_eq!(std::fs::read(dir.path().join("hosts.json")).unwrap(), bytes);
    }

    #[test]
    fn move_hosts_changes_folder_and_order_in_one_write() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        store.upsert_folder(hosts::fixtures::folder("f", None)).unwrap();
        store.upsert_host(hosts::fixtures::host("x", "x", "x", vec![]), vec![]).unwrap();
        store.upsert_host(hosts::fixtures::host("y", "y", "y", vec![]), vec![]).unwrap();
        store
            .move_hosts(&[
                HostMove { id: "x".into(), folder_id: Some("f".into()), order: 1 },
                HostMove { id: "y".into(), folder_id: Some("f".into()), order: 0 },
            ])
            .unwrap();
        let on_disk: HostsFile =
            serde_json::from_slice(&std::fs::read(dir.path().join("hosts.json")).unwrap()).unwrap();
        let get = |id: &str| on_disk.hosts.iter().find(|h| h.id == id).unwrap().clone();
        assert_eq!((get("x").folder_id.as_deref(), get("x").order), (Some("f"), 1));
        assert_eq!((get("y").folder_id.as_deref(), get("y").order), (Some("f"), 0));
    }

    #[test]
    fn upsert_keeps_unknown_fields_of_host_and_connections() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        let mut h = hosts::fixtures::host("h", "box", "a", vec![hosts::fixtures::vnc("c", None)]);
        store.upsert_host(h.clone(), vec![]).unwrap();
        let mut file = store.hosts();
        let stored = file.hosts.iter_mut().find(|x| x.id == "h").unwrap();
        stored.extra.insert("pinned".into(), serde_json::json!(true));
        stored.connections[0].extra.insert("note".into(), serde_json::json!("x"));
        store.replace_hosts(file).unwrap();

        h.name = "renamed".into();
        store.upsert_host(h, vec![]).unwrap();
        let after = store.hosts().hosts.into_iter().find(|x| x.id == "h").unwrap();
        assert_eq!(after.name, "renamed");
        assert_eq!(after.extra["pinned"], true);
        assert_eq!(after.connections[0].extra["note"], "x");
    }

    #[test]
    fn an_unknown_pending_delete_kind_does_not_break_the_file() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        store.upsert_host(hosts::fixtures::host("h", "web", "web.lan", vec![hosts::fixtures::vnc("c", None)]), vec![]).unwrap();
        drop(store);
        // A later version deleted something this one has no kind for.
        let path = dir.path().join("hosts.json");
        let item = json!({ "kind": "tunnel", "tunnel": { "id": "t" } });
        let mut raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        raw["pendingDeletes"] = json!([{ "id": "p1", "deletedAt": 1, "item": item }]);
        std::fs::write(&path, raw.to_string()).unwrap();

        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        assert!(!dir.path().join("hosts.json.corrupt").exists());
        assert!(store.hosts().hosts.iter().any(|h| h.id == "h"));
        assert!(matches!(store.undo_delete("p1"), Err(AppError::Invalid(_))));

        // A save keeps it as it was written, and the purge at start takes it like any other.
        store.upsert_host(hosts::fixtures::host("h2", "db", "db.lan", vec![]), vec![]).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["pendingDeletes"][0]["item"], item);
        store.purge_deletes(None).unwrap();
        assert!(store.hosts().pending_deletes.is_empty());
    }

    #[test]
    fn nested_login_fields_survive_an_edit() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        store
            .upsert_login(SavedLogin {
                id: "l".into(),
                name: "deploy".into(),
                username: "d".into(),
                auth_method: AuthMethod::Password,
                key_ids: vec![],
                secret_id: None,
                extra: Default::default(),
            })
            .unwrap();
        let own = || json!({ "kind": "own", "username": "u", "authMethod": "password", "keyIds": [], "secretId": null });
        let mut sent = serde_json::to_value(hosts::fixtures::host("h", "box", "a", vec![
            hosts::fixtures::ssh("c", 22, hosts::fixtures::own("u", None)),
            hosts::fixtures::ssh("d", 22, hosts::fixtures::own("u", None)),
            hosts::fixtures::ssh("e", 22, hosts::fixtures::own("u", None)),
        ]))
        .unwrap();
        sent["connections"][0]["kind"]["appearance"] = json!({ "colors": null, "fontFamily": null, "fontSize": null });
        sent["connections"][1]["kind"]["login"] = json!({ "kind": "saved", "loginId": "l" });
        sent["connections"][2]["kind"] = json!({ "type": "sftp", "own": { "port": 22, "login": own() }, "startFolder": null });
        // What a later version wrote: a field on each nested object.
        let mut later = sent.clone();
        later["connections"][0]["kind"]["login"]["note"] = json!("own");
        later["connections"][0]["kind"]["appearance"]["note"] = json!("look");
        later["connections"][1]["kind"]["login"]["note"] = json!("saved");
        later["connections"][2]["kind"]["own"]["note"] = json!("sftp");
        later["connections"][2]["kind"]["own"]["login"]["note"] = json!("sftp login");
        let mut file = store.hosts();
        file.hosts.push(serde_json::from_value(later).unwrap());
        store.replace_hosts(file).unwrap();

        // The editor sends the host back without them.
        let mut edit: Host = serde_json::from_value(sent).unwrap();
        edit.name = "renamed".into();
        store.upsert_host(edit, vec![]).unwrap();

        let after = serde_json::to_value(stored(&store, "h")).unwrap();
        assert_eq!(after["name"], "renamed");
        assert_eq!(after["connections"][0]["kind"]["login"]["note"], "own");
        assert_eq!(after["connections"][0]["kind"]["appearance"]["note"], "look");
        assert_eq!(after["connections"][1]["kind"]["login"]["note"], "saved");
        assert_eq!(after["connections"][2]["kind"]["own"]["note"], "sftp");
        assert_eq!(after["connections"][2]["kind"]["own"]["login"]["note"], "sftp login");
    }

    #[test]
    fn this_computer_keeps_its_name_and_place() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        store.upsert_folder(hosts::fixtures::folder("f", None)).unwrap();
        let mut pc = store.hosts().hosts.into_iter().find(|h| h.id == THIS_COMPUTER).unwrap();
        pc.name = "X".into();
        pc.folder_id = Some("f".into());
        store.upsert_host(pc, vec![]).unwrap();
        let after = store.hosts().hosts.into_iter().find(|h| h.id == THIS_COMPUTER).unwrap();
        assert_eq!(after.name, "This computer");
        assert_eq!(after.folder_id, None);
    }

    fn stored(store: &Store, id: &str) -> Host {
        store.hosts().hosts.into_iter().find(|h| h.id == id).unwrap()
    }

    fn conn_ids(store: &Store, id: &str) -> Vec<String> {
        stored(store, id).connections.into_iter().map(|c| c.id).collect()
    }

    #[test]
    fn a_deleted_host_keeps_its_secrets_until_purged() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        store.upsert_host(hosts::fixtures::host("h", "nas", "nas.local", vec![hosts::fixtures::vnc("c", Some("s1"))]), vec![]).unwrap();
        store.delete_host("h").unwrap();
        assert!(store.hosts().hosts.iter().all(|h| h.id != "h"));
        assert!(store.hosts().referenced_secrets().contains("s1"));
        let before = store.hosts();
        store.purge_deletes(None).unwrap();
        assert_eq!(crate::commands::store_cmds::secrets_to_delete(&before, &store.hosts()), vec!["s1".to_string()]);
    }

    #[test]
    fn undo_puts_a_host_back_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        store.upsert_host(hosts::fixtures::host("h", "nas", "nas.local", vec![hosts::fixtures::vnc("c", Some("s1"))]), vec![]).unwrap();
        let before = stored(&store, "h");
        let id = store.delete_host("h").unwrap();
        store.undo_delete(&id).unwrap();
        assert_eq!(stored(&store, "h"), before);
        assert!(store.hosts().pending_deletes.is_empty());
    }

    #[test]
    fn a_connection_left_out_of_a_save_becomes_a_pending_delete() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        let vnc = hosts::fixtures::vnc;
        let h = hosts::fixtures::host("h", "nas", "nas.local", vec![vnc("a", None), vnc("b", Some("s")), vnc("c", None)]);
        store.upsert_host(h.clone(), vec![]).unwrap();
        let mut edited = h;
        edited.connections.remove(1);
        let ids = store.upsert_host(edited, vec![]).unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(conn_ids(&store, "h"), ["a", "c"]);
        assert!(store.hosts().referenced_secrets().contains("s"));
        store.undo_delete(&ids[0]).unwrap();
        assert_eq!(conn_ids(&store, "h"), ["a", "b", "c"]);
    }

    #[test]
    fn a_moved_connection_lands_on_the_target_in_the_same_write() {
        use hosts::{ConnectionMove, MoveTarget, MoveToHost, NewHostTarget};
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        let vnc = hosts::fixtures::vnc;
        store.upsert_host(hosts::fixtures::host("h", "nas", "nas.local", vec![vnc("a", None), vnc("b", None), vnc("c", None)]), vec![]).unwrap();
        store.upsert_host(hosts::fixtures::host("t", "box", "box.lan", vec![]), vec![]).unwrap();
        let mut edited = stored(&store, "h");
        let b = edited.connections.remove(1);
        let c = edited.connections.remove(1);
        let ids = store
            .upsert_host(
                edited,
                vec![
                    ConnectionMove { connection: b, to: MoveTarget::Host(MoveToHost { host_id: "t".into() }) },
                    ConnectionMove {
                        connection: c,
                        to: MoveTarget::NewHost(NewHostTarget { name: "desk".into(), address: "desk.lan".into(), folder_id: None }),
                    },
                ],
            )
            .unwrap();
        assert!(ids.is_empty());
        assert_eq!(conn_ids(&store, "h"), ["a"]);
        assert_eq!(conn_ids(&store, "t"), ["b"]);
        let desk = store.hosts().hosts.into_iter().find(|h| h.name == "desk").unwrap();
        assert_eq!(desk.connections[0].id, "c");
        assert!(store.hosts().pending_deletes.is_empty());
        let on_disk: HostsFile =
            serde_json::from_slice(&std::fs::read(dir.path().join("hosts.json")).unwrap()).unwrap();
        assert_eq!(on_disk, store.hosts());
    }

    #[test]
    fn purging_a_duplicate_keeps_the_shared_secret() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        let vnc = hosts::fixtures::vnc;
        store.upsert_host(hosts::fixtures::host("h", "nas", "nas.local", vec![vnc("a", Some("s")), vnc("b", Some("s"))]), vec![]).unwrap();
        let mut edited = stored(&store, "h");
        edited.connections.remove(1);
        store.upsert_host(edited, vec![]).unwrap();
        let before = store.hosts();
        store.purge_deletes(None).unwrap();
        assert!(crate::commands::store_cmds::secrets_to_delete(&before, &store.hosts()).is_empty());
    }

    #[test]
    fn this_computer_cannot_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf(), &shells()).unwrap();
        assert!(matches!(store.delete_host(THIS_COMPUTER), Err(AppError::Invalid(_))));
    }
}
