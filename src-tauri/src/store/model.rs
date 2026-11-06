use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    Password,
    Key,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum IconRef {
    Builtin { name: String },
    Custom { path: String },
}

impl Default for IconRef {
    fn default() -> Self {
        IconRef::Builtin { name: "server".into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProfileAppearance {
    pub theme: Option<String>,
    pub font_family: Option<String>,
    pub font_size: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum TunnelKind {
    Local,
    Remote,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Tunnel {
    pub id: String,
    pub kind: TunnelKind,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: Option<String>,
    pub target_port: Option<u16>,
    pub auto_start: bool,
}

// One private key and its own optional passphrase (a vault reference, never the passphrase
// itself). A profile holds several and tries them in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProfileKey {
    pub path: String,
    pub secret_id: Option<String>,
}

// A saved SFTP-only connection: a file-transfer host with no shell, jump chain or tunnels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SftpProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    #[serde(default)]
    pub keys: Vec<ProfileKey>,
    pub secret_id: Option<String>,
    #[serde(default)]
    pub icon: IconRef,
    pub order: u32,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

// A saved S3 / S3-compatible connection. The secret access key never lives here - only a
// secret_id pointing at the encrypted vault. The access key id is not secret, so it stays inline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct S3Profile {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub port: Option<u16>,
    pub region: String,
    pub use_tls: bool,
    pub path_style: bool,
    pub access_key_id: String,
    pub secret_id: Option<String>,
    pub bucket: Option<String>,
    #[serde(default)]
    pub icon: IconRef,
    pub order: u32,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

// A saved VNC connection. The password is a secret_id into the vault, like the others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VncProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub secret_id: Option<String>,
    #[serde(default)]
    pub icon: IconRef,
    pub order: u32,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

// A saved RDP connection. The password is a secret_id into the vault, like the others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RdpProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub domain: Option<String>,
    pub secret_id: Option<String>,
    pub keyboard_layout: Option<u32>,
    #[serde(default = "default_true")]
    pub audio: bool,
    #[serde(default)]
    pub shared_folder: Option<SharedFolder>,
    #[serde(default)]
    pub gateway: Option<RdpGateway>,
    #[serde(default)]
    pub icon: IconRef,
    pub order: u32,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SharedFolder {
    pub path: String,
    pub name: String,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

// With `same_login` the gateway gets the RDP user name and password, and `username` and
// `secret_id` are not used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RdpGateway {
    pub host: String,
    pub port: u16,
    pub same_login: bool,
    pub username: String,
    pub secret_id: Option<String>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub icon: IconRef,
    pub order: u32,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub group_id: Option<String>,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    // Legacy single key: migrated into `keys` on load and cleared. Kept so older files parse.
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default)]
    pub keys: Vec<ProfileKey>,
    pub secret_id: Option<String>,
    #[serde(default)]
    pub icon: IconRef,
    pub order: u32,
    pub jump_host_id: Option<String>,
    #[serde(default)]
    pub tunnels: Vec<Tunnel>,
    #[serde(default)]
    pub appearance: Option<ProfileAppearance>,
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub theme: String,
    pub font_family: String,
    pub font_size: u16,
    pub color_scheme: String,
    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default = "default_background")]
    pub background: String,
    #[serde(default = "default_font_weight")]
    pub font_weight: String,
    #[serde(default = "default_line_height")]
    pub line_height: f32,
    #[serde(default)]
    pub letter_spacing: f32,
    #[serde(default = "default_cursor_style")]
    pub cursor_style: String,
    #[serde(default = "default_true")]
    pub cursor_blink: bool,
    #[serde(default = "default_scrollback")]
    pub scrollback: u32,
    #[serde(default)]
    pub vnc_clipboard_sync: bool,
    #[serde(default = "default_max_transfers")]
    pub max_concurrent_transfers: u32,
    #[serde(default)]
    pub copy_on_select: bool,
    #[serde(default)]
    pub right_click_paste: bool,
    // Action id -> chord, holding only the bindings the user changed from the defaults.
    #[serde(default)]
    pub hotkeys: std::collections::HashMap<String, String>,
    #[serde(default = "default_true")]
    pub restore_session: bool,
}

fn default_accent() -> String {
    "teal".into()
}
fn default_background() -> String {
    "teal".into()
}
fn default_font_weight() -> String {
    "normal".into()
}
fn default_line_height() -> f32 {
    1.0
}
fn default_cursor_style() -> String {
    "block".into()
}
fn default_true() -> bool {
    true
}
fn default_scrollback() -> u32 {
    10000
}
fn default_max_transfers() -> u32 {
    3
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "system".into(),
            font_family: "monospace".into(),
            font_size: 14,
            color_scheme: "default".into(),
            accent: default_accent(),
            background: default_background(),
            font_weight: default_font_weight(),
            line_height: default_line_height(),
            letter_spacing: 0.0,
            cursor_style: default_cursor_style(),
            cursor_blink: true,
            scrollback: default_scrollback(),
            vnc_clipboard_sync: false,
            max_concurrent_transfers: default_max_transfers(),
            copy_on_select: false,
            right_click_paste: false,
            hotkeys: Default::default(),
            restore_session: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProfileStore {
    pub version: u32,
    #[serde(default)]
    pub sftp_profiles: Vec<SftpProfile>,
    pub groups: Vec<Group>,
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub s3_profiles: Vec<S3Profile>,
    #[serde(default)]
    pub vnc_profiles: Vec<VncProfile>,
    #[serde(default)]
    pub rdp_profiles: Vec<RdpProfile>,
    // What a later version keeps in the file. It is written back as it was read: left out, the
    // first save by this version would delete it.
    #[serde(flatten)]
    #[specta(skip)]
    pub unknown: serde_json::Map<String, serde_json::Value>,
}

impl ProfileStore {
    pub const CURRENT_VERSION: u32 = 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profile() -> Profile {
        Profile {
            id: "p1".into(),
            name: "web-01".into(),
            group_id: Some("g1".into()),
            host: "10.0.0.1".into(),
            port: 22,
            username: "root".into(),
            auth_method: AuthMethod::Key,
            key_path: None,
            keys: vec![ProfileKey {
                path: "/home/me/.ssh/id_ed25519".into(),
                secret_id: None,
            }],
            secret_id: None,
            icon: IconRef::Builtin { name: "server".into() },
            order: 0,
            jump_host_id: None,
            tunnels: vec![],
            appearance: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn profile_round_trips_camel_case() {
        let p = sample_profile();
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"authMethod\":\"key\""));
        assert!(json.contains("\"groupId\":\"g1\""));
        let back: Profile = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn icon_ref_is_tagged() {
        let json = serde_json::to_string(&IconRef::Custom { path: "/x.png".into() }).unwrap();
        assert!(json.contains("\"kind\":\"custom\""));
        assert!(json.contains("\"path\":\"/x.png\""));
    }

    #[test]
    fn profile_tolerates_missing_optional_fields() {
        let minimal = r#"{"id":"p1","name":"n","groupId":null,"host":"h","port":22,
            "username":"u","authMethod":"password","keyPath":null,"secretId":null,
            "order":0,"jumpHostId":null}"#;
        let p: Profile = serde_json::from_str(minimal).unwrap();
        assert_eq!(p.icon, IconRef::default());
        assert!(p.tunnels.is_empty());
    }

    #[test]
    fn vnc_profiles_are_stored_camel_case() {
        let store = ProfileStore {
            vnc_profiles: vec![VncProfile {
                id: "v1".into(),
                name: "desk".into(),
                host: "10.0.0.5".into(),
                port: 5901,
                username: None,
                secret_id: Some("vault-1".into()),
                icon: IconRef::default(),
                order: 0,
                extra: Default::default(),
            }],
            ..Default::default()
        };
        let json = serde_json::to_value(&store).unwrap();
        assert_eq!(json["vncProfiles"][0]["secretId"], "vault-1");
        assert_eq!(json["vncProfiles"][0]["username"], serde_json::Value::Null);
        assert_eq!(serde_json::from_value::<ProfileStore>(json).unwrap(), store);
    }

    #[test]
    fn vnc_profile_tolerates_a_missing_icon() {
        let minimal = r#"{"id":"v1","name":"n","host":"h","port":5900,"username":"u",
            "secretId":null,"order":0}"#;
        let p: VncProfile = serde_json::from_str(minimal).unwrap();
        assert_eq!(p.icon, IconRef::default());
        assert_eq!(p.username.as_deref(), Some("u"));
    }

    #[test]
    fn rdp_profiles_are_stored_camel_case() {
        let store = ProfileStore {
            rdp_profiles: vec![RdpProfile {
                id: "r1".into(),
                name: "office".into(),
                host: "10.0.0.9".into(),
                port: 3389,
                username: "CORP\\faye".into(),
                domain: None,
                secret_id: Some("vault-1".into()),
                keyboard_layout: Some(0x407),
                audio: false,
                shared_folder: Some(SharedFolder {
                    path: "C:\\Users\\faye".into(),
                    name: "faye".into(),
                    extra: Default::default(),
                }),
                gateway: Some(RdpGateway {
                    host: "gw.corp".into(),
                    port: 443,
                    same_login: false,
                    username: "CORP\\gw".into(),
                    secret_id: Some("vault-2".into()),
                    extra: Default::default(),
                }),
                icon: IconRef::default(),
                order: 0,
                extra: Default::default(),
            }],
            ..Default::default()
        };
        let json = serde_json::to_value(&store).unwrap();
        assert_eq!(json["rdpProfiles"][0]["secretId"], "vault-1");
        assert_eq!(json["rdpProfiles"][0]["keyboardLayout"], 0x407);
        assert_eq!(json["rdpProfiles"][0]["domain"], serde_json::Value::Null);
        assert_eq!(json["rdpProfiles"][0]["audio"], false);
        assert_eq!(json["rdpProfiles"][0]["sharedFolder"]["path"], "C:\\Users\\faye");
        assert_eq!(json["rdpProfiles"][0]["gateway"]["sameLogin"], false);
        assert_eq!(json["rdpProfiles"][0]["gateway"]["secretId"], "vault-2");
        assert_eq!(serde_json::from_value::<ProfileStore>(json).unwrap(), store);
    }

    #[test]
    fn rdp_profile_tolerates_a_missing_icon() {
        let minimal = r#"{"id":"r1","name":"n","host":"h","port":3389,"username":"u",
            "domain":"corp","secretId":null,"keyboardLayout":null,"order":0}"#;
        let p: RdpProfile = serde_json::from_str(minimal).unwrap();
        assert_eq!(p.icon, IconRef::default());
        assert_eq!(p.domain.as_deref(), Some("corp"));
    }

    const PHASE_3_RDP: &str = r#"{"id":"r1","name":"n","host":"h","port":3389,"username":"u",
        "domain":null,"secretId":"vault-1","keyboardLayout":null,
        "icon":{"kind":"builtin","name":"server"},"order":0}"#;

    #[test]
    fn an_rdp_profile_from_before_sound_plays_sound_and_has_no_share_or_gateway() {
        let p: RdpProfile = serde_json::from_str(PHASE_3_RDP).unwrap();
        assert!(p.audio);
        assert_eq!(p.shared_folder, None);
        assert_eq!(p.gateway, None);
        assert!(p.extra.is_empty());
    }

    #[test]
    fn sound_folder_and_gateway_round_trip() {
        let mut json: serde_json::Value = serde_json::from_str(PHASE_3_RDP).unwrap();
        json["audio"] = false.into();
        json["sharedFolder"] = serde_json::json!({ "path": "/home/faye/share", "name": "faye" });
        json["gateway"] = serde_json::json!({ "host": "gw.corp", "port": 8443, "sameLogin": true,
            "username": "", "secretId": null });
        let p: RdpProfile = serde_json::from_value(json.clone()).unwrap();
        assert!(!p.audio);
        let folder = SharedFolder {
            path: "/home/faye/share".into(),
            name: "faye".into(),
            extra: Default::default(),
        };
        assert_eq!(p.shared_folder, Some(folder));
        let gateway = RdpGateway {
            host: "gw.corp".into(),
            port: 8443,
            same_login: true,
            username: String::new(),
            secret_id: None,
            extra: Default::default(),
        };
        assert_eq!(p.gateway, Some(gateway));
        assert!(p.extra.is_empty());
        assert_eq!(serde_json::to_value(&p).unwrap(), json);
    }

    #[test]
    fn fields_this_build_does_not_know_in_the_folder_and_the_gateway_are_written_back() {
        let mut json: serde_json::Value = serde_json::from_str(PHASE_3_RDP).unwrap();
        json["audio"] = true.into();
        json["sharedFolder"] = serde_json::json!({ "path": "/s", "name": "s", "readOnly": true });
        json["gateway"] = serde_json::json!({ "host": "gw", "port": 443, "sameLogin": false,
            "username": "u", "secretId": null, "auth": { "scheme": "ntlm" } });
        let p: RdpProfile = serde_json::from_value(json.clone()).unwrap();
        assert!(p.extra.is_empty());
        assert_eq!(p.shared_folder.as_ref().unwrap().extra["readOnly"], true);
        assert_eq!(p.gateway.as_ref().unwrap().extra["auth"]["scheme"], "ntlm");
        assert_eq!(serde_json::to_value(&p).unwrap(), json);
    }

    #[test]
    fn fields_this_build_does_not_know_still_land_in_extra() {
        let mut json: serde_json::Value = serde_json::from_str(PHASE_3_RDP).unwrap();
        json["audio"] = false.into();
        json["gateway"] = serde_json::json!({ "host": "gw", "port": 443, "sameLogin": true,
            "username": "", "secretId": null });
        json["scale"] = 1.5.into();
        let p: RdpProfile = serde_json::from_value(json).unwrap();
        assert_eq!(p.extra.keys().collect::<Vec<_>>(), ["scale"]);
        assert_eq!(p.extra["scale"], 1.5);
        let back = serde_json::to_value(&p).unwrap();
        assert_eq!(back["scale"], 1.5);
        assert_eq!(back["gateway"]["host"], "gw");
    }
}
