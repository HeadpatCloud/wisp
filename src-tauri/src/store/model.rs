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
    pub key_path: Option<String>,
    pub secret_id: Option<String>,
    #[serde(default)]
    pub icon: IconRef,
    pub order: u32,
    pub jump_host_id: Option<String>,
    #[serde(default)]
    pub tunnels: Vec<Tunnel>,
    #[serde(default)]
    pub appearance: Option<ProfileAppearance>,
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
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProfileStore {
    pub version: u32,
    pub groups: Vec<Group>,
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub s3_profiles: Vec<S3Profile>,
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
            key_path: Some("/home/me/.ssh/id_ed25519".into()),
            secret_id: None,
            icon: IconRef::Builtin { name: "server".into() },
            order: 0,
            jump_host_id: None,
            tunnels: vec![],
            appearance: None,
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
}
