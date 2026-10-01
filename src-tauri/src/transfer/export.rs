use std::collections::HashSet;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use zeroize::Zeroizing;

use super::bundle::{KeyFile, Payload};
use super::{Env, ExportOptions, ExportSelection};
use crate::store::model::{ProfileKey, ProfileStore};

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or("key").to_string()
}

pub fn build(
    data: &ProfileStore,
    env: &dyn Env,
    sel: &ExportSelection,
    opts: &ExportOptions,
) -> (Payload, Vec<String>) {
    let mut warnings = Vec::new();

    let mut profile_ids: HashSet<&str> = sel.profile_ids.iter().map(String::as_str).collect();
    // A profile is useless on the other machine without its jump chain.
    loop {
        let missing: Vec<&str> = data
            .profiles
            .iter()
            .filter(|p| profile_ids.contains(p.id.as_str()))
            .filter_map(|p| p.jump_host_id.as_deref())
            .filter(|j| !profile_ids.contains(j) && data.profiles.iter().any(|p| p.id == *j))
            .collect();
        if missing.is_empty() {
            break;
        }
        for id in missing {
            if !profile_ids.insert(id) {
                continue;
            }
            if let Some(p) = data.profiles.iter().find(|p| p.id == id) {
                warnings.push(format!("Also exported jump host \"{}\"", p.name));
            }
        }
    }

    let mut group_ids: HashSet<&str> = sel.group_ids.iter().map(String::as_str).collect();
    for p in data.profiles.iter().filter(|p| profile_ids.contains(p.id.as_str())) {
        group_ids.extend(p.group_id.as_deref());
    }
    let mut frontier: Vec<&str> = group_ids.iter().copied().collect();
    while let Some(id) = frontier.pop() {
        if let Some(parent) =
            data.groups.iter().find(|g| g.id == id).and_then(|g| g.parent_id.as_deref())
        {
            if group_ids.insert(parent) {
                frontier.push(parent);
            }
        }
    }

    let sftp_ids: HashSet<&str> = sel.sftp_ids.iter().map(String::as_str).collect();
    let s3_ids: HashSet<&str> = sel.s3_ids.iter().map(String::as_str).collect();
    let mut payload = Payload {
        groups: data.groups.iter().filter(|g| group_ids.contains(g.id.as_str())).cloned().collect(),
        profiles: data
            .profiles
            .iter()
            .filter(|p| profile_ids.contains(p.id.as_str()))
            .cloned()
            .collect(),
        sftp_profiles: data
            .sftp_profiles
            .iter()
            .filter(|p| sftp_ids.contains(p.id.as_str()))
            .cloned()
            .collect(),
        s3_profiles: data
            .s3_profiles
            .iter()
            .filter(|p| s3_ids.contains(p.id.as_str()))
            .cloned()
            .collect(),
        ..Default::default()
    };

    let mut owners: Vec<(&str, Option<&str>, &[ProfileKey])> = Vec::new();
    for p in &payload.profiles {
        owners.push((&p.name, p.secret_id.as_deref(), &p.keys));
    }
    for p in &payload.sftp_profiles {
        owners.push((&p.name, p.secret_id.as_deref(), &p.keys));
    }
    for p in &payload.s3_profiles {
        owners.push((&p.name, p.secret_id.as_deref(), &[]));
    }

    let mut secrets = Vec::new();
    let mut key_files = Vec::new();
    for (name, secret_id, keys) in owners {
        if opts.include_secrets {
            for id in
                secret_id.into_iter().chain(keys.iter().filter_map(|k| k.secret_id.as_deref()))
            {
                match env.secret(id).and_then(|v| String::from_utf8(v.to_vec()).ok()) {
                    Some(value) => secrets.push((id.to_string(), Zeroizing::new(value))),
                    None => warnings.push(format!("Couldn't read a saved password for \"{name}\"")),
                }
            }
        }
        if opts.include_keys {
            for k in keys {
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
        }
    }
    payload.secrets.extend(secrets);
    payload.key_files.extend(key_files);
    (payload, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::{AuthMethod, Group, IconRef, Profile, ProfileKey};
    use crate::transfer::MapEnv;

    fn group(id: &str, parent: Option<&str>) -> Group {
        Group {
            id: id.into(),
            name: id.into(),
            parent_id: parent.map(Into::into),
            icon: IconRef::default(),
            order: 0,
        }
    }

    fn profile(id: &str, group: Option<&str>, jump: Option<&str>) -> Profile {
        Profile {
            id: id.into(),
            name: id.into(),
            group_id: group.map(Into::into),
            host: "h".into(),
            port: 22,
            username: "u".into(),
            auth_method: AuthMethod::Key,
            key_path: None,
            keys: vec![ProfileKey {
                path: format!("/keys/{id}"),
                secret_id: Some(format!("pp-{id}")),
            }],
            secret_id: None,
            icon: IconRef::default(),
            order: 0,
            jump_host_id: jump.map(Into::into),
            tunnels: vec![],
            appearance: None,
        }
    }

    fn store() -> ProfileStore {
        ProfileStore {
            version: 1,
            groups: vec![group("root", None), group("child", Some("root")), group("other", None)],
            profiles: vec![
                profile("web", Some("child"), Some("bastion")),
                profile("bastion", None, None),
            ],
            sftp_profiles: vec![],
            s3_profiles: vec![],
        }
    }

    fn select(ids: &[&str]) -> ExportSelection {
        ExportSelection {
            profile_ids: ids.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn includes_ancestor_groups_and_jump_hosts() {
        let (payload, warnings) =
            build(&store(), &MapEnv::default(), &select(&["web"]), &ExportOptions::default());
        let groups: Vec<_> = payload.groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(groups, ["root", "child"]);
        let profiles: Vec<_> = payload.profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(profiles, ["web", "bastion"]);
        assert!(warnings.iter().any(|w| w.contains("bastion")));
    }

    #[test]
    fn shared_jump_host_is_warned_about_once() {
        let mut data = store();
        data.profiles.push(profile("db", None, Some("bastion")));
        let (payload, warnings) =
            build(&data, &MapEnv::default(), &select(&["web", "db"]), &ExportOptions::default());
        assert_eq!(payload.profiles.len(), 3);
        assert_eq!(warnings.iter().filter(|w| w.contains("bastion")).count(), 1);
    }

    #[test]
    fn secrets_and_keys_only_when_asked() {
        let mut env = MapEnv::default();
        env.secrets.insert("pp-web".into(), b"pass".to_vec());
        env.files.insert("/keys/web".into(), b"KEY".to_vec());
        let (plain, _) = build(&store(), &env, &select(&["web"]), &ExportOptions::default());
        assert!(plain.secrets.is_empty() && plain.key_files.is_empty());

        let opts = ExportOptions { include_secrets: true, include_keys: true };
        let (full, _) = build(&store(), &env, &select(&["web"]), &opts);
        assert_eq!(full.secrets.get("pp-web").map(|v| v.as_str()), Some("pass"));
        let key = full.key_files.get("/keys/web").unwrap();
        assert_eq!(key.file_name, "web");
        assert_eq!(STANDARD.decode(key.data.as_bytes()).unwrap(), b"KEY");
    }

    #[test]
    fn unreadable_key_and_binary_secret_become_warnings() {
        let mut env = MapEnv::default();
        env.secrets.insert("pp-web".into(), vec![0xff, 0xfe]);
        let opts = ExportOptions { include_secrets: true, include_keys: true };
        let (payload, warnings) = build(&store(), &env, &select(&["web"]), &opts);
        assert!(payload.secrets.get("pp-web").is_none());
        assert!(payload.key_files.is_empty());
        assert!(warnings.iter().any(|w| w.contains("/keys/web")));
        assert!(warnings.iter().any(|w| w.contains("web") && w.contains("password")));
    }

    #[test]
    fn explicitly_selected_empty_group_is_exported() {
        let sel = ExportSelection { group_ids: vec!["other".into()], ..Default::default() };
        let (payload, _) = build(&store(), &MapEnv::default(), &sel, &ExportOptions::default());
        assert_eq!(payload.groups.len(), 1);
        assert!(payload.profiles.is_empty());
    }
}
