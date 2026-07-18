use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::State;

use crate::error::{AppError, AppResult};

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ShellInfo {
    pub name: String,
    pub program: String,
}

fn edit_root() -> std::path::PathBuf {
    std::env::temp_dir().join("wisp-edit")
}

// Wipes leftovers from previous runs. Called at startup rather than on close so a file the
// user is still editing is never pulled out from under them mid-session.
#[tauri::command]
#[specta::specta]
pub fn clear_edit_temp() -> AppResult<()> {
    let root = edit_root();
    if root.exists() {
        std::fs::remove_dir_all(&root).map_err(|e| AppError::Io(e.to_string()))?;
    }
    Ok(())
}

// Scratch path for "edit remote file": a per-edit directory keeps the original filename
// intact so the re-upload lands back on the same remote name.
#[tauri::command]
#[specta::specta]
pub fn edit_temp_path(file_name: String) -> AppResult<String> {
    let name = std::path::Path::new(&file_name)
        .file_name()
        .ok_or_else(|| AppError::Io(format!("bad file name: {file_name}")))?;
    let dir = edit_root().join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir).map_err(|e| AppError::Io(e.to_string()))?;
    Ok(dir.join(name).to_string_lossy().into_owned())
}

// Seconds since the epoch, or 0 when the file is missing - the caller polls this to notice
// that the external editor saved. f64 because specta won't export u64 to TypeScript.
#[tauri::command]
#[specta::specta]
pub fn file_mtime(path: String) -> AppResult<f64> {
    let Ok(meta) = std::fs::metadata(&path) else {
        return Ok(0.0);
    };
    let secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    Ok(secs)
}

fn on_path(name: &str) -> Option<String> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(windows)]
fn find_git_bash() -> Option<String> {
    // git.exe lives in <Git>\cmd; the bash binary is in <Git>\bin\bash.exe.
    let git = on_path("git.exe")?;
    let bash = std::path::Path::new(&git).parent()?.parent()?.join("bin").join("bash.exe");
    bash.is_file().then(|| bash.to_string_lossy().into_owned())
}

// Detect the interactive shells available on this machine.
#[tauri::command]
#[specta::specta]
pub fn list_shells() -> Vec<ShellInfo> {
    let mut shells = Vec::new();
    #[cfg(windows)]
    {
        if let Some(p) = on_path("pwsh.exe") {
            shells.push(ShellInfo { name: "PowerShell".into(), program: p });
        }
        if let Some(p) = on_path("powershell.exe") {
            shells.push(ShellInfo { name: "Windows PowerShell".into(), program: p });
        }
        shells.push(ShellInfo { name: "Command Prompt".into(), program: "cmd.exe".into() });
        if let Some(p) = find_git_bash() {
            shells.push(ShellInfo { name: "Git Bash".into(), program: p });
        }
        if let Some(p) = on_path("wsl.exe") {
            shells.push(ShellInfo { name: "WSL".into(), program: p });
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(sh) = std::env::var("SHELL") {
            shells.push(ShellInfo { name: "Default shell".into(), program: sh });
        }
        for (name, p) in [("Bash", "/bin/bash"), ("Zsh", "/bin/zsh"), ("Fish", "/usr/bin/fish")] {
            if std::path::Path::new(p).exists() {
                shells.push(ShellInfo { name: name.into(), program: p.into() });
            }
        }
    }
    shells
}

pub struct LocalPty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

#[derive(Default)]
pub struct LocalSessions(pub Mutex<HashMap<String, LocalPty>>);

fn pty_err<E: std::fmt::Display>(e: E) -> AppError {
    AppError::Internal(format!("pty: {e}"))
}

fn poisoned() -> AppError {
    AppError::Internal("local lock poisoned".into())
}

#[tauri::command]
#[specta::specta]
pub async fn local_open(
    locals: State<'_, LocalSessions>,
    program: Option<String>,
    cols: u16,
    rows: u16,
    on_output: Channel<String>,
) -> AppResult<String> {
    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(pty_err)?;
    let cmd = match program {
        Some(p) if !p.is_empty() => CommandBuilder::new(p),
        _ => CommandBuilder::new_default_prog(),
    };
    let child = pair.slave.spawn_command(cmd).map_err(pty_err)?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(pty_err)?;
    let writer = pair.master.take_writer().map_err(pty_err)?;

    let id = uuid::Uuid::new_v4().to_string();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if on_output.send(STANDARD.encode(&buf[..n])).is_err() {
                        break;
                    }
                }
            }
        }
    });

    locals.0.lock().map_err(|_| poisoned())?.insert(id.clone(), LocalPty { master: pair.master, writer, child });
    Ok(id)
}

#[tauri::command]
#[specta::specta]
pub async fn local_write(
    locals: State<'_, LocalSessions>,
    id: String,
    data: Vec<u8>,
) -> AppResult<()> {
    let mut map = locals.0.lock().map_err(|_| poisoned())?;
    let pty = map.get_mut(&id).ok_or_else(|| AppError::NotFound(format!("local {id}")))?;
    pty.writer.write_all(&data).map_err(pty_err)?;
    pty.writer.flush().map_err(pty_err)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn local_resize(
    locals: State<'_, LocalSessions>,
    id: String,
    cols: u16,
    rows: u16,
) -> AppResult<()> {
    let map = locals.0.lock().map_err(|_| poisoned())?;
    let pty = map.get(&id).ok_or_else(|| AppError::NotFound(format!("local {id}")))?;
    pty.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(pty_err)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn local_close(locals: State<'_, LocalSessions>, id: String) -> AppResult<()> {
    if let Some(mut pty) = locals.0.lock().map_err(|_| poisoned())?.remove(&id) {
        let _ = pty.child.kill();
    }
    Ok(())
}
