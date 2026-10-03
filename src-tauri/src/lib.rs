//! Tauri application entry-point and all IPC commands.

mod audio;
mod error;
mod fs;
mod metadata;
mod mtp;

use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use tauri::Emitter;
use error::AppError;

// ─── Shared application state ─────────────────────────────────────────────────

pub struct AppState {
    pub mtp:   Arc<Mutex<mtp::MtpManager>>,
    pub audio: Arc<Mutex<audio::AudioPlayer>>,
    pub mount_search_cache: Arc<Mutex<HashMap<String, Vec<fs::IndexedTrack>>>>,
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

/// Scan for a connected MTP device and return its info (or null if none found).
#[tauri::command]
async fn scan_device(state: tauri::State<'_, AppState>) -> Result<Option<mtp::DeviceInfo>, AppError> {
    let mtp = Arc::clone(&state.mtp);
    tauri::async_runtime::spawn_blocking(move || mtp.lock().unwrap().scan())
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

/// Disconnect the current device.
#[tauri::command]
async fn disconnect_device(state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    state.mtp.lock().unwrap().disconnect();
    Ok(())
}

/// Check whether the currently connected MTP device is still visible.
#[tauri::command]
async fn mtp_device_present(state: tauri::State<'_, AppState>) -> Result<bool, AppError> {
    let mtp = Arc::clone(&state.mtp);
    tauri::async_runtime::spawn_blocking(move || {
        let manager = mtp.lock().unwrap();
        Ok(manager.is_connected() && manager.is_device_present())
    })
    .await
    .map_err(|e| AppError::Task(e.to_string()))?
}

/// List all tracks on the connected device.
#[tauri::command]
async fn get_device_tracks(state: tauri::State<'_, AppState>) -> Result<Vec<mtp::Track>, AppError> {
    let mtp = Arc::clone(&state.mtp);
    tauri::async_runtime::spawn_blocking(move || mtp.lock().unwrap().get_tracks())
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

/// Transfer one or more files to the device. Emits "transfer:progress" events.
/// Returns a map of { transfer_id: new_track_id } for successfully transferred files.
#[tauri::command]
async fn send_tracks(
    app:     tauri::AppHandle,
    state:   tauri::State<'_, AppState>,
    tracks:  Vec<mtp::SendTrackRequest>,
) -> Result<Vec<u32>, AppError> {
    let mtp        = Arc::clone(&state.mtp);
    let app_arc    = Arc::new(app);

    tauri::async_runtime::spawn_blocking(move || {
        let mut new_ids = Vec::new();

        for (i, req) in tracks.iter().enumerate() {
            let transfer_id = format!("transfer-{i}");
            let _ = app_arc.emit("transfer:start", serde_json::json!({
                "id":       transfer_id,
                "filename": std::path::Path::new(&req.path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(""),
                "index":    i,
                "total":    tracks.len(),
            }));

            let mut mtp_guard = mtp.lock().unwrap();

            match mtp_guard.send_track(req, Arc::clone(&app_arc), &transfer_id) {
                Ok(id) => {
                    new_ids.push(id);
                    let _ = app_arc.emit("transfer:done", serde_json::json!({
                        "id":       transfer_id,
                        "track_id": id,
                    }));
                }
                Err(e) => {
                    let _ = app_arc.emit("transfer:error", serde_json::json!({
                        "id":    transfer_id,
                        "error": e.to_string(),
                    }));
                }
            }
        }

        Ok(new_ids)
    })
    .await
    .map_err(|e| AppError::Task(e.to_string()))?
}

/// Delete a track from the device.
#[tauri::command]
async fn delete_track(
    state:    tauri::State<'_, AppState>,
    track_id: u32,
) -> Result<(), AppError> {
    let mtp = Arc::clone(&state.mtp);
    tauri::async_runtime::spawn_blocking(move || mtp.lock().unwrap().delete_track(track_id))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

/// Read metadata + cover art from a local audio file.
#[tauri::command]
async fn get_local_metadata(path: String) -> Result<metadata::TrackMetadata, AppError> {
    tauri::async_runtime::spawn_blocking(move || metadata::read(&path))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

#[tauri::command]
async fn get_local_file_size(path: String) -> Result<u64, AppError> {
    tauri::async_runtime::spawn_blocking(move || Ok(std::fs::metadata(path)?.len()))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

/// Read cover art for a single track, lazy-loaded on demand.
#[tauri::command]
async fn get_track_cover(path: String) -> Result<Option<String>, AppError> {
    let meta = tauri::async_runtime::spawn_blocking(move || metadata::read(&path))
        .await
        .map_err(|e| AppError::Task(e.to_string()))??;
    Ok(meta.cover_art)
}

/// Write updated metadata back to a local audio file.
#[tauri::command]
async fn update_local_metadata(
    path: String,
    meta: metadata::TrackMetadata,
) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || metadata::write(&path, &meta))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

/// Start previewing a local audio file.
#[tauri::command]
async fn preview_track(
    state: tauri::State<'_, AppState>,
    path:  String,
) -> Result<(), AppError> {
    state.audio.lock().unwrap().play(path)
}

/// Stop audio preview.
#[tauri::command]
async fn stop_preview(state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    state.audio.lock().unwrap().stop();
    Ok(())
}

// ─── Filesystem (mass-storage) device commands ───────────────────────────────

/// Detect mounted removable drives that contain audio files.
#[tauri::command]
async fn detect_mounts() -> Result<Vec<fs::MountDevice>, AppError> {
    tauri::async_runtime::spawn_blocking(fs::detect_mounts)
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

/// Scan a specific mount path and return device info.
#[tauri::command]
async fn scan_mount_device(mount_path: String) -> Result<Option<fs::MountDevice>, AppError> {
    tauri::async_runtime::spawn_blocking(move || fs::scan_mount(&mount_path))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

/// Check whether a mounted device path is still available.
#[tauri::command]
async fn mount_device_present(mount_path: String) -> Result<bool, AppError> {
    tauri::async_runtime::spawn_blocking(move || fs::mount_present(&mount_path))
        .await
        .map_err(|e| AppError::Task(e.to_string()))
}

/// List all audio tracks on a mounted device.
#[tauri::command]
async fn get_mount_tracks(mount_path: String) -> Result<Vec<fs::MountTrack>, AppError> {
    tauri::async_runtime::spawn_blocking(move || fs::get_tracks(&mount_path))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

/// Search the cached recursive library for a mounted device.
#[tauri::command]
async fn search_mount_tracks(
    state: tauri::State<'_, AppState>,
    mount_path: String,
    query: String,
    format: String,
) -> Result<Vec<fs::MountTrack>, AppError> {
    let cache = Arc::clone(&state.mount_search_cache);
    tauri::async_runtime::spawn_blocking(move || {
        fs::search_tracks(&mount_path, &query, &format, &cache)
    })
    .await
    .map_err(|e| AppError::Task(e.to_string()))?
}

#[tauri::command]
async fn clear_mount_search_cache(
    state: tauri::State<'_, AppState>,
    mount_path: String,
) -> Result<(), AppError> {
    state.mount_search_cache.lock().unwrap().remove(&mount_path);
    Ok(())
}

/// List contents of a single directory within a mount (folder view).
#[tauri::command]
async fn get_folder_contents(
    mount_path: String,
    sub_path:   String,
) -> Result<fs::FolderContents, AppError> {
    tauri::async_runtime::spawn_blocking(move || fs::list_directory(&mount_path, &sub_path))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

/// Copy a file onto a mounted device (supports progress events).
#[tauri::command]
async fn copy_to_device(
    app:         tauri::AppHandle,
    source:      String,
    dest_dir:    String,
    filename:    String,
    transfer_id: String,
) -> Result<String, AppError> {
    let app_arc = Arc::new(app);
    tauri::async_runtime::spawn_blocking(move || {
        fs::copy_to_device(&source, &dest_dir, &filename, &app_arc, &transfer_id)
    })
    .await
    .map_err(|e| AppError::Task(e.to_string()))?
}

/// Delete a file on a mounted device.
#[tauri::command]
async fn delete_mount_file(path: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || fs::delete_file(&path))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

#[tauri::command]
async fn rename_mount_files(
    mount_path: String,
    files: Vec<serde_json::Value>,
) -> Result<(), AppError> {
    let pairs = files
        .into_iter()
        .map(|file| {
            let old_path = file.get("oldPath").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let new_path = file.get("newPath").and_then(|v| v.as_str()).unwrap_or("").to_string();
            (old_path, new_path)
        })
        .collect::<Vec<_>>();
    tauri::async_runtime::spawn_blocking(move || fs::rename_files(&mount_path, &pairs))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

/// Recursively delete a folder on a mounted device.
#[tauri::command]
async fn delete_mount_folder(path: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || fs::delete_folder(&path))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

/// Get disk usage (total / used / free) for a mount path.
#[tauri::command]
async fn get_disk_usage(mount_path: String) -> Result<fs::DiskUsage, AppError> {
    tauri::async_runtime::spawn_blocking(move || fs::disk_usage(&mount_path))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

/// If path is an audio file, return it. If it's a directory, recursively
/// find all audio files inside. Used for drag-drop of folders.
#[tauri::command]
async fn expand_audio_path(path: String) -> Result<Vec<String>, AppError> {
    tauri::async_runtime::spawn_blocking(move || fs::expand_audio_path(&path))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

/// Create a directory and all parents on a mounted device.
#[tauri::command]
async fn create_dir_all(path: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || fs::create_dir_all(&path))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

#[tauri::command]
async fn sync_mount(mount_path: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || fs::sync_mount(&mount_path))
        .await
        .map_err(|e| AppError::Task(e.to_string()))?
}

// ─── App entry-point ─────────────────────────────────────────────────────────

pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            mtp:        Arc::new(Mutex::new(mtp::MtpManager::new())),
            audio:      Arc::new(Mutex::new(audio::AudioPlayer::new())),
            mount_search_cache: Arc::new(Mutex::new(HashMap::new())),
        })
        .invoke_handler(tauri::generate_handler![
            scan_device,
            disconnect_device,
            mtp_device_present,
            get_device_tracks,
            send_tracks,
            delete_track,
            get_local_metadata,
            get_local_file_size,
            get_track_cover,
            update_local_metadata,
            preview_track,
            stop_preview,
            detect_mounts,
            scan_mount_device,
            mount_device_present,
            get_mount_tracks,
            search_mount_tracks,
            clear_mount_search_cache,
            get_folder_contents,
            copy_to_device,
            delete_mount_file,
            rename_mount_files,
            delete_mount_folder,
            get_disk_usage,
            expand_audio_path,
            create_dir_all,
            sync_mount,
        ])
        .run(tauri::generate_context!())
        .expect("Error while running Snowsky Transfer");
}
