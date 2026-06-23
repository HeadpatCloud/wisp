use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};
use crate::ssh::ssh_config::parse_ssh_config;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportCandidate {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub key_path: Option<String>,
    pub jump_host_alias: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn import_ssh_config(
    app: AppHandle,
    path: Option<String>,
) -> AppResult<Vec<ImportCandidate>> {
    let home = app.path().home_dir().map_err(|e| AppError::Io(e.to_string()))?;
    let config_path = match path {
        Some(p) => PathBuf::from(p),
        None => home.join(".ssh").join("config"),
    };
    let text = std::fs::read_to_string(&config_path)
        .map_err(|e| AppError::Io(format!("{}: {e}", config_path.display())))?;

    let candidates = parse_ssh_config(&text)
        .into_iter()
        .map(|h| {
            let key_path = h.identity_file.map(|f| {
                if let Some(rest) = f.strip_prefix("~/").or_else(|| f.strip_prefix("~\\")) {
                    home.join(rest).to_string_lossy().into_owned()
                } else {
                    f
                }
            });
            ImportCandidate {
                host: h.host_name.unwrap_or_else(|| h.name.clone()),
                port: h.port.unwrap_or(22),
                username: h.user.unwrap_or_default(),
                key_path,
                jump_host_alias: h.proxy_jump,
                name: h.name,
            }
        })
        .collect();
    Ok(candidates)
}
