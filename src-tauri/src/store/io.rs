use std::io::Write;
use std::path::Path;

use serde::{de::DeserializeOwned, Serialize};

use crate::error::{AppError, AppResult};

pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> AppResult<T> {
    if !path.exists() {
        return Ok(T::default());
    }
    let bytes = std::fs::read(path)?;
    match serde_json::from_slice(&bytes) {
        Ok(value) => Ok(value),
        Err(_) => {
            // Corrupt/unparseable config would otherwise brick startup. Preserve
            // it next to the original so it's recoverable, and start from default.
            if let Some(name) = path.file_name() {
                let backup = path.with_file_name(format!("{}.corrupt", name.to_string_lossy()));
                let _ = std::fs::rename(path, backup);
            }
            Ok(T::default())
        }
    }
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Io("invalid file path".into()))?
        .to_string_lossy();
    let tmp = path.with_file_name(format!("{file_name}.tmp"));
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(&bytes)?;
    f.sync_all()?; // flush to disk before the rename so a crash can't leave a half-written file
    drop(f);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::Settings;

    #[test]
    fn missing_file_yields_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        let s: Settings = read_json(&path).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn corrupt_file_is_set_aside_and_yields_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"{ not valid json at all").unwrap();
        let s: Settings = read_json(&path).unwrap();
        assert_eq!(s, Settings::default());
        assert!(dir.path().join("settings.json.corrupt").exists());
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/settings.json");
        let mut s = Settings::default();
        s.font_size = 18;
        write_json_atomic(&path, &s).unwrap();
        let back: Settings = read_json(&path).unwrap();
        assert_eq!(back.font_size, 18);
    }
}
