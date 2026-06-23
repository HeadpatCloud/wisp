use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};

const MAX_ICON_BYTES: u64 = 2 * 1024 * 1024;

fn icon_extension(source_path: &str) -> AppResult<String> {
    let ext = Path::new(source_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .ok_or_else(|| AppError::Io("icon has no file extension".into()))?;
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => Ok(ext),
        _ => Err(AppError::Io(format!("unsupported icon type: {ext}"))),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn import_icon(app: AppHandle, source_path: String) -> AppResult<String> {
    let ext = icon_extension(&source_path)?;
    if std::fs::metadata(&source_path)?.len() > MAX_ICON_BYTES {
        return Err(AppError::Io("icon file is too large (max 2 MB)".into()));
    }
    let icons_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| AppError::Io(e.to_string()))?
        .join("icons");
    std::fs::create_dir_all(&icons_dir)?;
    let name = format!("{}.{ext}", uuid::Uuid::new_v4());
    std::fs::copy(&source_path, icons_dir.join(&name))?;
    Ok(format!("icons/{name}"))
}

#[tauri::command]
#[specta::specta]
pub async fn read_icon(app: AppHandle, rel_path: String) -> AppResult<String> {
    if rel_path.contains("..") {
        return Err(AppError::Io("invalid icon path".into()));
    }
    let path = app
        .path()
        .app_config_dir()
        .map_err(|e| AppError::Io(e.to_string()))?
        .join(&rel_path);
    let bytes = std::fs::read(&path)?;
    let ext = Path::new(&rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_lowercase();
    let mime = match ext.as_str() {
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    };
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_known_image_extensions_lowercased() {
        assert_eq!(icon_extension("/a/logo.PNG").unwrap(), "png");
        assert_eq!(icon_extension("/a/pic.jpeg").unwrap(), "jpeg");
        assert_eq!(icon_extension("/a/v.svg").unwrap(), "svg");
    }

    #[test]
    fn rejects_non_image_and_extensionless() {
        assert!(icon_extension("/a/evil.exe").is_err());
        assert!(icon_extension("/a/noext").is_err());
    }
}
