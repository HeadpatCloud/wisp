mod commands;
mod error;
mod ftp;
mod s3;
mod sftp;
mod ssh;
mod store;
mod tunnel;
mod vault;
mod vnc;

use std::sync::{Arc, Mutex};

use commands::ftp_cmds;
use commands::icon_cmds;
use commands::import_cmds;
use commands::local_cmds;
use commands::s3_cmds;
use commands::sftp_cmds;
use commands::ssh_cmds;
use commands::store_cmds;
use commands::tunnel_cmds;
use commands::vault_cmds;
use commands::vnc_cmds;
use error::AppError;
use error::AppResult;
use ssh::known_hosts::KnownHosts;
use vault::Vault;
use specta_typescript::Typescript;
use tauri::Manager;
use tauri_specta::{collect_commands, collect_events, Builder};

use store::Store;

#[tauri::command]
#[specta::specta]
fn health_check() -> AppResult<String> {
    Ok("ok".to_string())
}

fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            health_check,
            store_cmds::list_groups,
            store_cmds::list_profiles,
            store_cmds::get_settings,
            store_cmds::upsert_group,
            store_cmds::delete_group,
            store_cmds::upsert_profile,
            store_cmds::delete_profile,
            store_cmds::set_settings,
            store_cmds::export_profiles,
            store_cmds::import_profiles,
            store_cmds::list_s3_profiles,
            store_cmds::upsert_s3_profile,
            store_cmds::delete_s3_profile,
            vault_cmds::vault_status,
            vault_cmds::set_secret,
            vault_cmds::delete_secret,
            vault_cmds::has_secret,
            vault_cmds::vault_unlock,
            vault_cmds::vault_change_password,
            ssh_cmds::ssh_connect,
            ssh_cmds::ssh_write,
            ssh_cmds::ssh_resize,
            ssh_cmds::ssh_disconnect,
            ssh_cmds::trust_host_key,
            sftp_cmds::sftp_list,
            sftp_cmds::sftp_stat,
            sftp_cmds::sftp_mkdir,
            sftp_cmds::sftp_rename,
            sftp_cmds::sftp_remove,
            sftp_cmds::sftp_upload,
            sftp_cmds::sftp_download,
            sftp_cmds::sftp_cancel,
            sftp_cmds::sftp_connect,
            sftp_cmds::sftp_disconnect,
            ftp_cmds::ftp_connect,
            ftp_cmds::ftp_list,
            ftp_cmds::ftp_exists,
            ftp_cmds::ftp_mkdir,
            ftp_cmds::ftp_rename,
            ftp_cmds::ftp_remove,
            ftp_cmds::ftp_upload,
            ftp_cmds::ftp_download,
            ftp_cmds::ftp_cancel,
            ftp_cmds::ftp_disconnect,
            s3_cmds::s3_connect,
            s3_cmds::s3_list_buckets,
            s3_cmds::s3_list,
            s3_cmds::s3_upload,
            s3_cmds::s3_download,
            s3_cmds::s3_delete,
            s3_cmds::s3_rename,
            s3_cmds::s3_mkdir,
            s3_cmds::s3_cancel,
            s3_cmds::s3_disconnect,
            tunnel_cmds::tunnel_start,
            tunnel_cmds::tunnel_stop,
            tunnel_cmds::tunnel_list,
            import_cmds::import_ssh_config,
            icon_cmds::import_icon,
            icon_cmds::read_icon,
            local_cmds::list_shells,
            local_cmds::local_open,
            local_cmds::local_write,
            local_cmds::local_resize,
            local_cmds::local_close,
            vnc_cmds::vnc_open,
            vnc_cmds::vnc_pointer,
            vnc_cmds::vnc_key,
            vnc_cmds::vnc_cut_text,
            vnc_cmds::vnc_close,
        ])
        .events(collect_events![
            ssh_cmds::SshStatus,
            tunnel::TunnelStatus,
            vnc_cmds::VncClipboard
        ])
}

// WebKitGTK's GPU paths (DMABUF + accelerated compositing) blank to a white/grey window
// on many modern Linux setups (Wayland, recent Mesa, Nvidia). Force the software fallbacks
// and, for AppImages on Wayland, re-exec with the system libwayland-client preloaded so the
// bundled copy doesn't lose a load-order conflict (the classic white-screen / EGL_BAD_PARAMETER).
#[cfg(target_os = "linux")]
fn linux_render_fixes() {
    for var in ["WEBKIT_DISABLE_DMABUF_RENDERER", "WEBKIT_DISABLE_COMPOSITING_MODE"] {
        if std::env::var_os(var).is_none() {
            std::env::set_var(var, "1");
        }
    }

    use std::os::unix::process::CommandExt;
    if std::env::var_os("APPIMAGE").is_none() || std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return;
    }
    // Loop guard for the child we re-exec below.
    if std::env::var_os("WISP_WAYLAND_PRELOADED").is_some() {
        return;
    }
    let existing = std::env::var("LD_PRELOAD").unwrap_or_default();
    if existing.contains("libwayland-client") {
        return;
    }
    let Some(lib) = [
        "/usr/lib/x86_64-linux-gnu/libwayland-client.so.0",
        "/usr/lib64/libwayland-client.so.0",
        "/usr/lib/libwayland-client.so.0",
    ]
    .into_iter()
    .find(|p| std::path::Path::new(*p).exists()) else {
        return;
    };
    let preload = if existing.is_empty() {
        lib.to_string()
    } else {
        format!("{lib}:{existing}")
    };
    let err = std::process::Command::new(std::env::current_exe().expect("current exe"))
        .args(std::env::args_os().skip(1))
        .env("LD_PRELOAD", preload)
        .env("WISP_WAYLAND_PRELOADED", "1")
        .exec();
    // exec() only returns on failure - fall through and run unpreloaded.
    eprintln!("wisp: wayland preload re-exec failed: {err}");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    linux_render_fixes();

    let builder = specta_builder();

    #[cfg(debug_assertions)]
    builder
        .export(Typescript::default(), "../src/bindings.ts")
        .expect("failed to export typescript bindings");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let dir = app.path().app_config_dir()?;
            let store = Store::load(dir.clone())
                .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;
            app.manage(Mutex::new(store));
            let vault_path = dir.join("vault.enc");
            let vault = match Vault::open_from_keychain(vault_path.clone()) {
                Ok(v) => v,
                Err(AppError::Keyring(_)) => Vault::open_locked(vault_path)
                    .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?,
                Err(e) => return Err(Box::<dyn std::error::Error>::from(e.to_string())),
            };
            app.manage(Mutex::new(vault));
            let known_hosts = KnownHosts::load(dir.join("known_hosts.json"))
                .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;
            app.manage(ssh_cmds::KnownHostsState(Arc::new(std::sync::Mutex::new(known_hosts))));
            app.manage(ssh_cmds::Sessions::default());
            app.manage(sftp_cmds::SftpSessions::default());
            app.manage(sftp_cmds::SftpConns::default());
            app.manage(sftp_cmds::Transfers::default());
            app.manage(local_cmds::LocalSessions::default());
            app.manage(ftp_cmds::FtpSessions::default());
            app.manage(ftp_cmds::FtpTransfers::default());
            app.manage(s3_cmds::S3Sessions::default());
            app.manage(s3_cmds::S3Transfers::default());
            app.manage(vnc_cmds::VncSessions::default());
            app.manage(tunnel_cmds::Tunnels::default());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod bindings_export {
    use super::specta_builder;
    use specta_typescript::Typescript;

    // Regenerate src/bindings.ts headlessly (no GUI):
    //   cargo test --manifest-path src-tauri/Cargo.toml export_bindings
    #[test]
    fn export_bindings() {
        specta_builder()
            .export(Typescript::default(), "../src/bindings.ts")
            .expect("failed to export typescript bindings");
    }
}
