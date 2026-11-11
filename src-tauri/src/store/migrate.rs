use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_json::{Map, Value};
use specta::Type;
use uuid::Uuid;

use super::hosts::{
    ConnAppearance, Connection, ConnectionKind, Folder, Host, HostsFile, LocalConn, Login,
    OwnLogin, OwnSftp, RdpConn, RdpGatewayConn, S3Conn, SftpConn, SshConn, SshKey, VncConn,
    THIS_COMPUTER,
};
use super::model::{AuthMethod, Group, IconRef, ProfileKey, ProfileStore};
use super::normalize_keys;
use crate::commands::local_cmds::ShellInfo;
use crate::net::address_key;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    pub profiles: u32,
    pub hosts: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Family {
    Ssh,
    Vnc,
    Rdp,
}

impl Family {
    fn default_port(self) -> u16 {
        match self {
            Family::Ssh => 22,
            Family::Vnc => 5900,
            Family::Rdp => 3389,
        }
    }
}

// One old profile, in the order the 0.5.0 sidebar showed them.
struct Item {
    id: String,
    name: String,
    address: String,
    port: u16,
    // None for S3, which never merges.
    family: Option<Family>,
    folder_id: Option<String>,
    icon: IconRef,
    kind: ConnectionKind,
    extra: Map<String, Value>,
}

#[derive(Default)]
struct Keys {
    list: Vec<SshKey>,
    by_path: HashMap<String, usize>,
}

impl Keys {
    fn ids(&mut self, keys: &[ProfileKey]) -> Vec<String> {
        keys.iter().map(|k| self.id(k)).collect()
    }

    fn id(&mut self, key: &ProfileKey) -> String {
        let i = match self.by_path.get(&key.path) {
            Some(&i) => i,
            None => {
                self.list.push(SshKey {
                    id: Uuid::new_v4().to_string(),
                    path: key.path.clone(),
                    secret_id: None,
                    extra: Default::default(),
                });
                self.by_path.insert(key.path.clone(), self.list.len() - 1);
                self.list.len() - 1
            }
        };
        let entry = &mut self.list[i];
        if entry.secret_id.is_none() {
            entry.secret_id = key.secret_id.clone();
        }
        entry.id.clone()
    }
}

fn own(username: &str, auth_method: AuthMethod, key_ids: Vec<String>, secret_id: Option<String>) -> Login {
    Login::Own(OwnLogin { username: username.to_string(), auth_method, key_ids, secret_id, extra: Default::default() })
}

// Old files can hold a parent that is gone or a loop of parents. Either would leave folders
// that never show, so such a folder becomes a top-level one.
fn folders(groups: &[Group]) -> Vec<Folder> {
    let ids: HashSet<&str> = groups.iter().map(|g| g.id.as_str()).collect();
    let parents: HashMap<&str, &str> = groups
        .iter()
        .filter_map(|g| g.parent_id.as_deref().filter(|p| ids.contains(p)).map(|p| (g.id.as_str(), p)))
        .collect();
    let in_a_loop = |start: &str| {
        let mut seen = HashSet::new();
        let mut current = start;
        while let Some(&parent) = parents.get(current) {
            if parent == start {
                return true;
            }
            if !seen.insert(parent) {
                return false;
            }
            current = parent;
        }
        false
    };
    groups
        .iter()
        .map(|g| Folder {
            id: g.id.clone(),
            name: g.name.clone(),
            parent_id: parents
                .get(g.id.as_str())
                .filter(|_| !in_a_loop(&g.id))
                .map(|p| p.to_string()),
            icon: g.icon.clone(),
            order: g.order,
            extra: g.extra.clone(),
        })
        .collect()
}

fn items(old: &ProfileStore, keys: &mut Keys) -> Vec<Item> {
    let known: HashSet<&str> = old.groups.iter().map(|g| g.id.as_str()).collect();
    let mut groups: Vec<&Group> = old.groups.iter().collect();
    groups.sort_by_key(|g| g.order);
    let by_order = |mut v: Vec<_>| {
        v.sort_by_key(|p: &&super::model::Profile| p.order);
        v
    };
    let mut ssh = Vec::new();
    for g in &groups {
        ssh.extend(by_order(old.profiles.iter().filter(|p| p.group_id.as_deref() == Some(&g.id)).collect()));
    }
    ssh.extend(by_order(
        old.profiles
            .iter()
            .filter(|p| p.group_id.as_deref().is_none_or(|g| !known.contains(g)))
            .collect(),
    ));

    let mut items = Vec::new();
    for p in ssh {
        let mut p = p.clone();
        normalize_keys(&mut p);
        let key_ids = keys.ids(&p.keys);
        items.push(Item {
            id: p.id.clone(),
            name: p.name.clone(),
            address: p.host.clone(),
            port: p.port,
            family: Some(Family::Ssh),
            folder_id: p.group_id.clone().filter(|g| known.contains(g.as_str())),
            icon: p.icon.clone(),
            kind: ConnectionKind::Ssh(SshConn {
                port: p.port,
                login: own(&p.username, p.auth_method, key_ids, p.secret_id.clone()),
                jump_host_id: p.jump_host_id.clone(),
                tunnels: p.tunnels.clone(),
                appearance: p.appearance.as_ref().map(|a| ConnAppearance {
                    colors: a.theme.clone(),
                    font_family: a.font_family.clone(),
                    font_size: a.font_size,
                    extra: Default::default(),
                }),
                extra: Default::default(),
            }),
            extra: p.extra.clone(),
        });
    }

    let mut sftp: Vec<_> = old.sftp_profiles.iter().collect();
    sftp.sort_by_key(|p| p.order);
    for p in sftp {
        let key_ids = keys.ids(&p.keys);
        items.push(Item {
            id: p.id.clone(),
            name: p.name.clone(),
            address: p.host.clone(),
            port: p.port,
            family: Some(Family::Ssh),
            folder_id: None,
            icon: p.icon.clone(),
            kind: ConnectionKind::Sftp(SftpConn {
                own: Some(OwnSftp {
                    port: p.port,
                    login: own(&p.username, p.auth_method, key_ids, p.secret_id.clone()),
                    extra: Default::default(),
                }),
                start_folder: None,
                extra: Default::default(),
            }),
            extra: p.extra.clone(),
        });
    }

    let mut s3: Vec<_> = old.s3_profiles.iter().collect();
    s3.sort_by_key(|p| p.order);
    for p in s3 {
        items.push(Item {
            id: p.id.clone(),
            name: p.name.clone(),
            address: p.endpoint.clone(),
            port: p.port.unwrap_or(0),
            family: None,
            folder_id: None,
            icon: p.icon.clone(),
            kind: ConnectionKind::S3(S3Conn {
                port: p.port,
                region: p.region.clone(),
                use_tls: p.use_tls,
                path_style: p.path_style,
                access_key_id: p.access_key_id.clone(),
                secret_id: p.secret_id.clone(),
                bucket: p.bucket.clone(),
                extra: Default::default(),
            }),
            extra: p.extra.clone(),
        });
    }

    let mut vnc: Vec<_> = old.vnc_profiles.iter().collect();
    vnc.sort_by_key(|p| p.order);
    for p in vnc {
        items.push(Item {
            id: p.id.clone(),
            name: p.name.clone(),
            address: p.host.clone(),
            port: p.port,
            family: Some(Family::Vnc),
            folder_id: None,
            icon: p.icon.clone(),
            kind: ConnectionKind::Vnc(VncConn {
                port: p.port,
                username: p.username.clone(),
                secret_id: p.secret_id.clone(),
                extra: Default::default(),
            }),
            extra: p.extra.clone(),
        });
    }

    let mut rdp: Vec<_> = old.rdp_profiles.iter().collect();
    rdp.sort_by_key(|p| p.order);
    for p in rdp {
        items.push(Item {
            id: p.id.clone(),
            name: p.name.clone(),
            address: p.host.clone(),
            port: p.port,
            family: Some(Family::Rdp),
            folder_id: None,
            icon: p.icon.clone(),
            kind: ConnectionKind::Rdp(RdpConn {
                port: p.port,
                login: own(&p.username, AuthMethod::Password, vec![], p.secret_id.clone()),
                domain: p.domain.clone(),
                keyboard_layout: p.keyboard_layout,
                audio: p.audio,
                shared_folder: p.shared_folder.clone(),
                gateway: p.gateway.as_ref().map(|g| RdpGatewayConn {
                    host: g.host.clone(),
                    port: g.port,
                    login: (!g.same_login).then(|| {
                        own(&g.username, AuthMethod::Password, vec![], g.secret_id.clone())
                    }),
                    extra: g.extra.clone(),
                }),
                extra: Default::default(),
            }),
            extra: p.extra.clone(),
        });
    }
    items
}

// Groups item indexes into hosts: S3 alone, the rest by address, with each family's other
// ports split off.
fn host_groups(items: &[Item]) -> Vec<Vec<usize>> {
    let mut groups = Vec::new();
    let mut addresses: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if item.family.is_none() {
            groups.push(vec![i]);
            continue;
        }
        let key = address_key(&item.address);
        match addresses.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(i),
            None => addresses.push((key, vec![i])),
        }
    }
    for (_, members) in addresses {
        let mut main: HashMap<Family, u16> = HashMap::new();
        for &i in &members {
            let family = items[i].family.expect("only S3 has no family");
            let uses_default = members
                .iter()
                .any(|&j| items[j].family == Some(family) && items[j].port == family.default_port());
            main.entry(family).or_insert(if uses_default { family.default_port() } else { items[i].port });
        }
        let mut main_host = Vec::new();
        let mut others: Vec<(Family, u16, Vec<usize>)> = Vec::new();
        for &i in &members {
            let family = items[i].family.expect("only S3 has no family");
            if items[i].port == main[&family] {
                main_host.push(i);
            } else {
                match others.iter_mut().find(|(f, p, _)| *f == family && *p == items[i].port) {
                    Some((_, _, list)) => list.push(i),
                    None => others.push((family, items[i].port, vec![i])),
                }
            }
        }
        groups.push(main_host);
        groups.extend(others.into_iter().map(|(_, _, list)| list));
    }
    groups.sort_by_key(|g| g[0]);
    groups
}

pub fn this_computer(shells: &[ShellInfo]) -> Host {
    Host {
        id: THIS_COMPUTER.into(),
        name: "This computer".into(),
        address: String::new(),
        folder_id: None,
        tag_ids: vec![],
        icon: IconRef::Builtin { name: "terminal".into() },
        color: None,
        order: 0,
        connections: shells
            .iter()
            .map(|s| Connection {
                id: Uuid::new_v4().to_string(),
                label: None,
                address: None,
                kind: ConnectionKind::Local(LocalConn {
                    program: s.program.clone(),
                    start_folder: None,
                    extra: Default::default(),
                }),
                extra: Default::default(),
            })
            .collect(),
        extra: Default::default(),
    }
}

// Builds hosts from the 0.5.0 profiles. Reads `old` only: nothing in the vault, the key files or
// the icons changes, so 0.5.0 still finds everything it refers to.
pub fn migrate(old: &ProfileStore, shells: &[ShellInfo]) -> (HostsFile, MigrationReport) {
    let mut keys = Keys::default();
    let items = items(old, &mut keys);
    let groups = host_groups(&items);
    let report = MigrationReport { profiles: items.len() as u32, hosts: groups.len() as u32 };

    let mut used = HashSet::new();
    let mut items: Vec<Option<Item>> = items.into_iter().map(Some).collect();
    let mut hosts = Vec::new();
    if !shells.is_empty() {
        hosts.push(this_computer(shells));
    }
    for (n, members) in groups.iter().enumerate() {
        let first = items[members[0]].as_ref().expect("each item is in one host");
        let name = first.name.clone();
        let address = first.address.clone();
        let folder_id = first.folder_id.clone();
        let icon = first.icon.clone();
        let connections = members
            .iter()
            .map(|&i| {
                let item = items[i].take().expect("each item is in one host");
                let id = if used.insert(item.id.clone()) {
                    item.id
                } else {
                    let fresh = Uuid::new_v4().to_string();
                    used.insert(fresh.clone());
                    fresh
                };
                Connection {
                    id,
                    label: (item.name != name).then_some(item.name),
                    address: None,
                    kind: item.kind,
                    extra: item.extra,
                }
            })
            .collect();
        hosts.push(Host {
            id: Uuid::new_v4().to_string(),
            name,
            address,
            folder_id,
            tag_ids: vec![],
            icon,
            color: None,
            order: n as u32 + 1,
            connections,
            extra: Default::default(),
        });
    }

    let file = HostsFile {
        version: HostsFile::CURRENT_VERSION,
        folders: folders(&old.groups),
        hosts,
        logins: vec![],
        keys: keys.list,
        tags: vec![],
        pending_deletes: vec![],
        unknown: Default::default(),
    };
    (file, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::{
        AuthMethod, Group, IconRef, Profile, ProfileAppearance, ProfileKey, ProfileStore,
        RdpProfile, S3Profile, SftpProfile, Tunnel, TunnelKind, VncProfile,
    };

    fn ssh(id: &str, name: &str, host: &str, port: u16, group: Option<&str>, order: u32) -> Profile {
        Profile {
            id: id.into(),
            name: name.into(),
            group_id: group.map(Into::into),
            host: host.into(),
            port,
            username: "admin".into(),
            auth_method: AuthMethod::Password,
            key_path: None,
            keys: vec![],
            secret_id: Some(format!("pw-{id}")),
            icon: IconRef::default(),
            order,
            jump_host_id: None,
            tunnels: vec![],
            appearance: None,
            extra: Default::default(),
        }
    }

    fn sftp(id: &str, name: &str, host: &str, port: u16, order: u32) -> SftpProfile {
        SftpProfile {
            id: id.into(),
            name: name.into(),
            host: host.into(),
            port,
            username: "files".into(),
            auth_method: AuthMethod::Password,
            keys: vec![],
            secret_id: None,
            icon: IconRef::default(),
            order,
            extra: Default::default(),
        }
    }

    fn s3(id: &str, name: &str, endpoint: &str, order: u32) -> S3Profile {
        S3Profile {
            id: id.into(),
            name: name.into(),
            endpoint: endpoint.into(),
            port: None,
            region: "eu".into(),
            use_tls: true,
            path_style: false,
            access_key_id: "AK".into(),
            secret_id: None,
            bucket: Some("b".into()),
            icon: IconRef::default(),
            order,
            extra: Default::default(),
        }
    }

    fn vnc(id: &str, name: &str, host: &str, port: u16, order: u32) -> VncProfile {
        VncProfile {
            id: id.into(),
            name: name.into(),
            host: host.into(),
            port,
            username: None,
            secret_id: None,
            icon: IconRef::default(),
            order,
            extra: Default::default(),
        }
    }

    fn rdp(id: &str, name: &str, host: &str, port: u16, order: u32) -> RdpProfile {
        RdpProfile {
            id: id.into(),
            name: name.into(),
            host: host.into(),
            port,
            username: "dev".into(),
            domain: None,
            secret_id: None,
            keyboard_layout: None,
            audio: true,
            shared_folder: None,
            gateway: None,
            icon: IconRef::default(),
            order,
            extra: Default::default(),
        }
    }

    fn group(id: &str, parent: Option<&str>, order: u32) -> Group {
        Group {
            id: id.into(),
            name: id.into(),
            parent_id: parent.map(Into::into),
            icon: IconRef::default(),
            order,
            extra: Default::default(),
        }
    }

    fn shell(name: &str, program: &str) -> ShellInfo {
        ShellInfo { name: name.into(), program: program.into() }
    }

    fn kinds(host: &Host) -> Vec<&'static str> {
        host.connections
            .iter()
            .map(|c| match &c.kind {
                ConnectionKind::Ssh(_) => "ssh",
                ConnectionKind::Sftp(_) => "sftp",
                ConnectionKind::Ftp(_) => "ftp",
                ConnectionKind::S3(_) => "s3",
                ConnectionKind::Rdp(_) => "rdp",
                ConnectionKind::Vnc(_) => "vnc",
                ConnectionKind::Local(_) => "local",
            })
            .collect()
    }

    fn named<'a>(file: &'a HostsFile, name: &str) -> &'a Host {
        file.hosts.iter().find(|h| h.name == name).unwrap_or_else(|| panic!("no host {name}"))
    }

    fn ssh_port(c: &Connection) -> u16 {
        match &c.kind {
            ConnectionKind::Ssh(s) => s.port,
            other => panic!("not ssh: {other:?}"),
        }
    }

    #[test]
    fn merges_profiles_on_one_address_into_one_host() {
        let old = ProfileStore {
            groups: vec![group("g1", None, 0)],
            profiles: vec![ssh("p1", "nas", "nas.local", 22, Some("g1"), 0)],
            sftp_profiles: vec![sftp("f1", "nas files", "NAS.local.", 22, 0)],
            vnc_profiles: vec![vnc("v1", "nas", "nas.local", 5900, 0)],
            rdp_profiles: vec![rdp("r1", "desk", "nas.local", 3389, 0)],
            ..Default::default()
        };
        let (file, report) = migrate(&old, &[]);
        assert_eq!(file.hosts.len(), 1);
        let host = &file.hosts[0];
        assert_eq!(host.name, "nas");
        assert_eq!(host.folder_id.as_deref(), Some("g1"));
        assert_eq!(kinds(host), ["ssh", "sftp", "vnc", "rdp"]);
        let labels: Vec<_> = host.connections.iter().map(|c| c.label.as_deref()).collect();
        assert_eq!(labels, [None, Some("nas files"), None, Some("desk")]);
        assert_eq!(report, MigrationReport { profiles: 4, hosts: 1 });
    }

    #[test]
    fn a_second_ssh_port_becomes_its_own_host() {
        let old = ProfileStore {
            profiles: vec![ssh("a", "a", "h", 22, None, 0), ssh("b", "b", "h", 2222, None, 1)],
            rdp_profiles: vec![rdp("c", "c", "h", 3389, 0)],
            ..Default::default()
        };
        let (file, _) = migrate(&old, &[]);
        assert_eq!(file.hosts.len(), 2);
        let a = named(&file, "a");
        assert_eq!(kinds(a), ["ssh", "rdp"]);
        assert_eq!(ssh_port(&a.connections[0]), 22);
        let b = named(&file, "b");
        assert_eq!(kinds(b), ["ssh"]);
        assert_eq!(ssh_port(&b.connections[0]), 2222);
    }

    #[test]
    fn without_the_default_port_the_first_profile_sets_the_main_port() {
        let old = ProfileStore {
            profiles: vec![ssh("x", "x", "h", 2200, None, 0), ssh("y", "y", "h", 2201, None, 1)],
            ..Default::default()
        };
        let (file, _) = migrate(&old, &[]);
        assert_eq!(file.hosts.len(), 2);
        assert_eq!(ssh_port(&named(&file, "x").connections[0]), 2200);
        assert_eq!(ssh_port(&named(&file, "y").connections[0]), 2201);
    }

    #[test]
    fn s3_profiles_never_merge() {
        let old = ProfileStore {
            profiles: vec![ssh("p", "box", "s3.local", 22, None, 0)],
            s3_profiles: vec![s3("s1", "one", "s3.local", 0), s3("s2", "two", "s3.local", 1)],
            ..Default::default()
        };
        let (file, report) = migrate(&old, &[]);
        assert_eq!(file.hosts.len(), 3);
        assert_eq!(kinds(named(&file, "one")), ["s3"]);
        assert_eq!(named(&file, "two").address, "s3.local");
        assert_eq!(report.hosts, 3);
    }

    #[test]
    fn name_and_folder_come_from_the_ssh_profile() {
        let old = ProfileStore {
            groups: vec![group("g", None, 0)],
            profiles: vec![ssh("p", "box", "10.0.0.5", 22, Some("g"), 3)],
            vnc_profiles: vec![vnc("v", "desk", "10.0.0.5", 5900, 0)],
            ..Default::default()
        };
        let (file, _) = migrate(&old, &[]);
        assert_eq!(file.hosts.len(), 1);
        assert_eq!(file.hosts[0].name, "box");
        assert_eq!(file.hosts[0].folder_id.as_deref(), Some("g"));
        assert_eq!(kinds(&file.hosts[0]), ["ssh", "vnc"]);
    }

    #[test]
    fn connection_ids_are_profile_ids() {
        let old = ProfileStore {
            profiles: vec![ssh("a", "box", "h", 22, None, 0)],
            vnc_profiles: vec![vnc("a", "screen", "other", 5900, 0)],
            ..Default::default()
        };
        let (file, _) = migrate(&old, &[]);
        assert_eq!(named(&file, "box").connections[0].id, "a");
        let vnc_id = &named(&file, "screen").connections[0].id;
        assert_ne!(vnc_id, "a");
        assert!(uuid::Uuid::parse_str(vnc_id).is_ok());
    }

    #[test]
    fn jump_hosts_tunnels_and_looks_carry_over() {
        let tunnel = Tunnel {
            id: "t1".into(),
            kind: TunnelKind::Local,
            bind_host: "127.0.0.1".into(),
            bind_port: 8080,
            target_host: Some("127.0.0.1".into()),
            target_port: Some(80),
            auto_start: true,
        };
        let mut target = ssh("t", "target", "10.0.0.9", 22, None, 1);
        target.jump_host_id = Some("bastion-id".into());
        target.tunnels = vec![tunnel.clone()];
        target.appearance = Some(ProfileAppearance {
            theme: Some("dracula".into()),
            font_family: Some("Mono".into()),
            font_size: Some(15),
        });
        let old = ProfileStore {
            profiles: vec![ssh("bastion-id", "bastion", "bastion.corp", 22, None, 0), target],
            ..Default::default()
        };
        let (file, _) = migrate(&old, &[]);
        let ConnectionKind::Ssh(conn) = &named(&file, "target").connections[0].kind else {
            panic!("not ssh")
        };
        assert_eq!(conn.jump_host_id.as_deref(), Some("bastion-id"));
        assert_eq!(conn.tunnels, [tunnel]);
        let look = conn.appearance.as_ref().unwrap();
        assert_eq!(look.colors.as_deref(), Some("dracula"));
        assert_eq!(look.font_family.as_deref(), Some("Mono"));
        assert_eq!(look.font_size, Some(15));
    }

    #[test]
    fn a_dangling_jump_host_survives_migration() {
        let mut p = ssh("p", "box", "h", 22, None, 0);
        p.jump_host_id = Some("gone".into());
        let old = ProfileStore { profiles: vec![p], ..Default::default() };
        let (file, _) = migrate(&old, &[]);
        let ConnectionKind::Ssh(conn) = &file.hosts[0].connections[0].kind else {
            panic!("not ssh")
        };
        assert_eq!(conn.jump_host_id.as_deref(), Some("gone"));
    }

    #[test]
    fn keys_are_listed_once_with_the_first_passphrase() {
        let key = |secret: &str| ProfileKey { path: "/k/id".into(), secret_id: Some(secret.into()) };
        let mut one = ssh("one", "one", "h1", 22, None, 0);
        one.auth_method = AuthMethod::Key;
        one.secret_id = None;
        one.keys = vec![key("s1")];
        let mut two = ssh("two", "two", "h2", 22, None, 1);
        two.auth_method = AuthMethod::Key;
        two.secret_id = None;
        two.keys = vec![key("s2")];
        let mut legacy = ssh("old", "old", "h3", 22, None, 2);
        legacy.auth_method = AuthMethod::Key;
        legacy.key_path = Some("/k/old".into());
        legacy.secret_id = Some("s3".into());
        let old = ProfileStore { profiles: vec![one, two, legacy], ..Default::default() };
        let (file, _) = migrate(&old, &[]);

        assert_eq!(file.keys.len(), 2);
        let shared = file.keys.iter().find(|k| k.path == "/k/id").unwrap();
        assert_eq!(shared.secret_id.as_deref(), Some("s1"));
        let folded = file.keys.iter().find(|k| k.path == "/k/old").unwrap();
        assert_eq!(folded.secret_id.as_deref(), Some("s3"));

        let login = |name: &str| match &named(&file, name).connections[0].kind {
            ConnectionKind::Ssh(SshConn { login: Login::Own(own), .. }) => own.clone(),
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(login("one").key_ids, [shared.id.clone()]);
        assert_eq!(login("two").key_ids, [shared.id.clone()]);
        assert_eq!(login("old").key_ids, [folded.id.clone()]);
        assert_eq!(login("old").secret_id, None);
    }

    #[test]
    fn groups_become_nested_folders() {
        let old = ProfileStore {
            groups: vec![group("g1", None, 0), group("g2", Some("g1"), 1)],
            ..Default::default()
        };
        let (file, _) = migrate(&old, &[]);
        let g2 = file.folders.iter().find(|f| f.id == "g2").unwrap();
        assert_eq!(g2.parent_id.as_deref(), Some("g1"));
        assert_eq!(g2.order, 1);
    }

    #[test]
    fn a_folder_loop_or_missing_parent_becomes_top_level() {
        let old = ProfileStore {
            groups: vec![
                group("a", Some("b"), 0),
                group("b", Some("a"), 1),
                group("c", Some("gone"), 2),
                group("d", Some("a"), 3),
            ],
            ..Default::default()
        };
        let (file, _) = migrate(&old, &[]);
        let parent = |id: &str| file.folders.iter().find(|f| f.id == id).unwrap().parent_id.clone();
        assert_eq!(parent("a"), None);
        assert_eq!(parent("b"), None);
        assert_eq!(parent("c"), None);
        assert_eq!(parent("d").as_deref(), Some("a"));
    }

    #[test]
    fn unknown_profile_fields_land_on_the_connection() {
        let mut p = ssh("p", "box", "h", 22, None, 0);
        p.extra.insert("x".into(), serde_json::json!(1));
        let old = ProfileStore { profiles: vec![p], ..Default::default() };
        let (file, _) = migrate(&old, &[]);
        assert_eq!(file.hosts[0].connections[0].extra["x"], 1);
    }

    #[test]
    fn this_computer_is_seeded_from_shells() {
        let shells = [shell("PowerShell", "pwsh.exe"), shell("Git Bash", "bash.exe")];
        let old = ProfileStore {
            profiles: vec![ssh("p", "box", "h", 22, None, 0)],
            ..Default::default()
        };
        let (file, report) = migrate(&old, &shells);
        let pc = file.hosts.iter().find(|h| h.id == THIS_COMPUTER).unwrap();
        assert_eq!(pc.name, "This computer");
        let programs: Vec<_> = pc
            .connections
            .iter()
            .map(|c| match &c.kind {
                ConnectionKind::Local(l) => l.program.as_str(),
                other => panic!("not local: {other:?}"),
            })
            .collect();
        assert_eq!(programs, ["pwsh.exe", "bash.exe"]);
        assert_eq!(report.hosts, 1);
    }

    #[test]
    fn no_shells_means_no_this_computer() {
        let (file, _) = migrate(&ProfileStore::default(), &[]);
        assert!(file.hosts.iter().all(|h| h.id != THIS_COMPUTER));
    }

    #[test]
    fn address_key_matches_trust_normalisation() {
        assert_eq!(crate::net::address_key("NAS.local."), "nas.local");
        assert_eq!(crate::net::address_key("[::1]"), "::1");
        assert_eq!(crate::net::address_key("0:0::1"), "::1");
    }
}
