use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use super::model::Settings;

// User themes and imported terminal schemes. The frontend owns their shape; the backend only
// keeps them and makes the two tint themes when it migrates a 0.5.0 setup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ThemesFile {
    pub version: u32,
    #[serde(default)]
    #[specta(type = Vec<specta_typescript::Unknown>)]
    pub themes: Vec<Value>,
    #[serde(default)]
    #[specta(type = Vec<specta_typescript::Unknown>)]
    pub terminal_schemes: Vec<Value>,
}

impl Default for ThemesFile {
    fn default() -> Self {
        ThemesFile { version: 1, themes: Vec::new(), terminal_schemes: Vec::new() }
    }
}

// The 0.5.0 background tints and their hues.
const TINTS: [(&str, u32); 7] = [
    ("teal", 195),
    ("slate", 255),
    ("indigo", 285),
    ("plum", 330),
    ("rose", 15),
    ("sand", 75),
    ("forest", 150),
];

// 0.5.0's light/dark and tint become Paper and Graphite; a tint other than teal becomes a pair
// of user themes based on them, so the look stays the same.
pub fn migrate(settings: &mut Settings, themes: &mut ThemesFile) {
    settings.light_theme = "paper".into();
    settings.dark_theme = "graphite".into();
    let Some(&(tint, hue)) = TINTS.iter().find(|(name, _)| *name == settings.background) else {
        return;
    };
    if tint == "teal" {
        return;
    }
    let label = format!("{}{}", tint[..1].to_uppercase(), &tint[1..]);
    for (base, kind, name) in [("paper", "light", "Paper"), ("graphite", "dark", "Graphite")] {
        let id = format!("{base}-{tint}");
        if !themes.themes.iter().any(|t| t["id"] == id.as_str()) {
            themes.themes.push(json!({
                "id": id,
                "name": format!("{name} · {label}"),
                "kind": kind,
                "tint": hue,
                "base": base,
            }));
        }
        if kind == "light" {
            settings.light_theme = id;
        } else {
            settings.dark_theme = id;
        }
    }
}
