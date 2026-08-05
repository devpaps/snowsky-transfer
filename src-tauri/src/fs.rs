//! Filesystem-based device support for mounted USB mass storage devices.
//!
//! When the Echo Mini (or similar device) mounts as a regular filesystem
//! instead of MTP, we read tracks directly from the mounted path.

use std::path::Path;
use std::sync::Arc;
use tauri::Emitter;

use crate::error::AppError;
use crate::metadata;

// ─── Disk usage ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiskUsage {
    pub total_bytes: u64,
    pub used_bytes:  u64,
    pub free_bytes:  u64,
}

/// Get filesystem statistics for the given mount path using statvfs.
pub fn disk_usage(mount_path: &str) -> Result<DiskUsage, AppError> {
    let path_c = std::ffi::CString::new(mount_path)
        .map_err(|e| AppError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e)))?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::statvfs(path_c.as_ptr(), &mut stat) };
    if ret != 0 {
        return Err(AppError::Io(std::io::Error::last_os_error()));
    }
    let block_size = stat.f_frsize as u64;
    let total = stat.f_blocks * block_size;
    let free  = stat.f_bfree * block_size;
    // "available" (f_bavail) is what non-root users can use, but for display
    // purposes used = total - free (free includes reserved blocks).
    Ok(DiskUsage {
        total_bytes: total,
        used_bytes:  total.saturating_sub(free),
        free_bytes:  free,
    })
}

// ─── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct MountDevice {
    pub name:        String,
    pub mount_path:  String,
    pub track_count: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MountTrack {
    pub id:           String,
    pub path:         String,
    pub title:        Option<String>,
    pub artist:       Option<String>,
    pub album:        Option<String>,
    pub genre:        Option<String>,
    pub year:         Option<u32>,
    pub track_number: Option<u32>,
    pub duration_ms:  u32,
    pub filetype:     String,
    pub file_size:    u64,
    pub cover_art:    Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DirEntry {
    pub name:       String,
    pub rel_path:   String,
    pub file_count: usize,
    pub audio_size_bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FolderContents {
    pub directories: Vec<DirEntry>,
    pub files:       Vec<MountTrack>,
    pub current_rel: String,
    pub parent_rel:  Option<String>,
}

// ─── Constants ───────────────────────────────────────────────────────────────

const AUDIO_EXTS: &[&str] = &["mp3", "flac", "ogg", "wav", "m4a", "aac"];

// ─── Public API ──────────────────────────────────────────────────────────────

/// Detect mounted removable drives under `/run/media/$USER/` that contain
/// audio files. Returns an empty vec if nothing is found.
pub fn detect_mounts() -> Result<Vec<MountDevice>, AppError> {
    let user = std::env::var("USER").unwrap_or_else(|_| "devpaps".into());
    let base = Path::new("/run/media").join(&user);

    if !base.exists() {
        return Ok(Vec::new());
    }

    let mut devices = Vec::new();
    for entry in std::fs::read_dir(&base)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                let count = count_audio_files(&path);
                if count > 0 {
                    devices.push(MountDevice {
                        name: name.to_string(),
                        mount_path: path.to_string_lossy().to_string(),
                        track_count: count,
                    });
                }
            }
        }
    }
    Ok(devices)
}

/// Return info for a single mount path (checks it exists + counts tracks).
pub fn scan_mount(mount_path: &str) -> Result<Option<MountDevice>, AppError> {
    let path = Path::new(mount_path);
    if !path.exists() || !path.is_dir() {
        return Ok(None);
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Device")
        .to_string();
    let count = count_audio_files(path);
    Ok(Some(MountDevice {
        name,
        mount_path: mount_path.to_string(),
        track_count: count,
    }))
}

/// Recursively scan a mount path for audio tracks, reading metadata.
pub fn get_tracks(mount_path: &str) -> Result<Vec<MountTrack>, AppError> {
    let base = Path::new(mount_path);
    if !base.exists() {
        return Ok(Vec::new());
    }
    let mut tracks = Vec::new();
    walk_audio_files(base, base, &mut tracks)?;
    Ok(tracks)
}

/// Copy a file onto the mounted device. `dest_dir` is the mount path root;
/// the file is placed at `dest_dir / filename`. Emits transfer progress events.
pub fn copy_to_device(
    source:      &str,
    dest_dir:    &str,
    filename:    &str,
    app_handle:  &Arc<tauri::AppHandle>,
    transfer_id: &str,
) -> Result<String, AppError> {
    let dest = Path::new(dest_dir).join(filename);
    if dest.exists() {
        return Err(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("File already exists: {}", dest.display()),
        )));
    }
    let src_meta = std::fs::metadata(source)?;
    let total = src_meta.len();

    // Chunked copy so we can report progress
    const CHUNK_SIZE: u64 = 512 * 1024; // 512 KiB
    let mut src_file = std::fs::File::open(source)?;
    let temp_name = format!(
        ".{}.{}.part",
        filename,
        std::process::id(),
    );
    let temp = Path::new(dest_dir).join(temp_name);
    let mut dst_file = std::fs::File::create(&temp)?;
    let mut buf = vec![0u8; CHUNK_SIZE as usize];
    let mut copied = 0u64;

    use std::io::Read;
    use std::io::Write;

    let result = loop {
        let n = src_file.read(&mut buf)?;
        if n == 0 {
            break Ok(());
        }
        dst_file.write_all(&buf[..n])?;
        copied += n as u64;
        let pct = if total > 0 {
            (copied * 100 / total) as u8
        } else {
            100
        };
        let _ = app_handle.emit("transfer:progress", serde_json::json!({
            "id":      transfer_id,
            "percent": pct,
        }));
    };
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    dst_file.sync_all()?;
    if let Err(error) = std::fs::rename(&temp, &dest) {
        let _ = std::fs::remove_file(&temp);
        return Err(error.into());
    }

    Ok(dest.to_string_lossy().to_string())
}

/// Delete a file on the mounted device.
pub fn delete_file(path: &str) -> Result<(), AppError> {
    std::fs::remove_file(path)?;
    Ok(())
}

/// Recursively delete a directory on the mounted device.
pub fn delete_folder(path: &str) -> Result<(), AppError> {
    std::fs::remove_dir_all(path)?;
    Ok(())
}

/// Create a directory and all parents on the mounted device.
pub fn create_dir_all(path: &str) -> Result<(), AppError> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

pub fn sync_mount(mount_path: &str) -> Result<(), AppError> {
    let sync_status = std::process::Command::new("sync").status()?;
    if !sync_status.success() {
        return Err(AppError::Task("Filesystem sync failed".into()));
    }
    let unmount_status = std::process::Command::new("umount")
        .arg(mount_path)
        .status()?;
    if !unmount_status.success() {
        return Err(AppError::Task(
            "Could not unmount the device. Close files using it and try again.".into(),
        ));
    }
    Ok(())
}

/// List the contents of a single directory level within a mount.
/// Returns subdirectories and audio files (with metadata) at this level only.
pub fn list_directory(mount_root: &str, sub_path: &str) -> Result<FolderContents, AppError> {
    let dir = Path::new(mount_root).join(sub_path.trim_start_matches('/'));
    if !dir.exists() || !dir.is_dir() {
        return Ok(FolderContents {
            directories: Vec::new(),
            files: Vec::new(),
            current_rel: sub_path.to_string(),
            parent_rel: None,
        });
    }

    let mut directories = Vec::new();
    let mut files = Vec::new();

    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        if name.starts_with('.') {
            continue;
        }

        let rel = if sub_path.is_empty() || sub_path == "/" {
            name.clone()
        } else {
            format!("{}/{}", sub_path.trim_end_matches('/'), name)
        };

        if path.is_dir() {
            let file_count = count_audio_files(&path);
            let audio_size_bytes = audio_directory_size(&path);
            directories.push(DirEntry {
                name,
                rel_path: rel,
                file_count,
                audio_size_bytes,
            });
        } else if is_audio_ext(&path) {
            let file_size = std::fs::metadata(&path)?.len();
            let meta = metadata::read(&path.to_string_lossy()).ok();

            files.push(MountTrack {
                id: rel.clone(),
                path: path.to_string_lossy().to_string(),
                title: meta.as_ref().and_then(|m| m.title.clone()),
                artist: meta.as_ref().and_then(|m| m.artist.clone()),
                album: meta.as_ref().and_then(|m| m.album.clone()),
                genre: meta.as_ref().and_then(|m| m.genre.clone()),
                year: meta.as_ref().and_then(|m| m.year),
                track_number: meta.as_ref().and_then(|m| m.track_number),
                duration_ms: meta.as_ref().map(|m| m.duration_ms).unwrap_or(0),
                filetype: path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_string(),
                file_size,
                cover_art: meta.and_then(|m| m.cover_art),
            });
        }
    }

    directories.sort_by(|a, b| a.name.cmp(&b.name));
    files.sort_by(|a, b| {
        a.title
            .as_deref()
            .unwrap_or("")
            .cmp(b.title.as_deref().unwrap_or(""))
    });

    let parent_rel = if sub_path.is_empty() || sub_path == "/" {
        None
    } else {
        Path::new(sub_path)
            .parent()
            .and_then(|p| {
                let s = p.to_string_lossy();
                if s.is_empty() { None } else { Some(s.to_string()) }
            })
    };

    Ok(FolderContents {
        directories,
        files,
        current_rel: sub_path.to_string(),
        parent_rel,
    })
}

// ─── Internal helpers ────────────────────────────────────────────────────────

fn walk_audio_files(base: &Path, dir: &Path, tracks: &mut Vec<MountTrack>) -> Result<(), AppError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_audio_files(base, &path, tracks)?;
        } else if is_audio_ext(&path) {
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let file_size = std::fs::metadata(&path)?.len();
            let meta = metadata::read(&path.to_string_lossy()).ok();

            tracks.push(MountTrack {
                id: rel.clone(),
                path: path.to_string_lossy().to_string(),
                title: meta.as_ref().and_then(|m| m.title.clone()),
                artist: meta.as_ref().and_then(|m| m.artist.clone()),
                album: meta.as_ref().and_then(|m| m.album.clone()),
                genre: meta.as_ref().and_then(|m| m.genre.clone()),
                year: meta.as_ref().and_then(|m| m.year),
                track_number: meta.as_ref().and_then(|m| m.track_number),
                duration_ms: meta.as_ref().map(|m| m.duration_ms).unwrap_or(0),
                filetype: path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_string(),
                file_size,
                cover_art: meta.and_then(|m| m.cover_art),
            });
        }
    }
    Ok(())
}

fn is_audio_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e))
        .unwrap_or(false)
}

/// If `path` is an audio file, return `vec![path]`.
/// If it's a directory, recursively find all audio files inside.
/// Otherwise return an empty vec.
pub fn expand_audio_path(path: &str) -> Result<Vec<String>, AppError> {
    let p = Path::new(path);
    if !p.exists() {
        return Ok(Vec::new());
    }
    if p.is_dir() {
        let files = walk_files_sorted(p)?;
        Ok(files)
    } else if is_audio_ext(p) {
        Ok(vec![path.to_string()])
    } else {
        Ok(Vec::new())
    }
}

fn walk_files_sorted(dir: &Path) -> Result<Vec<String>, AppError> {
    let mut entries: Vec<(String, Option<u32>)> = Vec::new();
    collect(dir, &mut entries)?;
    entries.sort_by(|(p1, t1), (p2, t2)| {
        match (t1, t2) {
            (Some(a), Some(b)) => a.cmp(b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => p1.cmp(p2),
        }
    });
    Ok(entries.into_iter().map(|(p, _)| p).collect())
}

fn collect(dir: &Path, out: &mut Vec<(String, Option<u32>)>) -> Result<(), AppError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if is_audio_ext(&path) {
            let track = metadata::read(&path.to_string_lossy())
                .ok()
                .and_then(|m| m.track_number);
            out.push((path.to_string_lossy().to_string(), track));
        }
    }
    Ok(())
}

fn count_audio_files(dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += count_audio_files(&path);
            } else if is_audio_ext(&path) {
                count += 1;
            }
        }
    }
    count
}

fn audio_directory_size(dir: &Path) -> u64 {
    let mut size = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                size += audio_directory_size(&path);
            } else if is_audio_ext(&path) {
                let metadata = match std::fs::metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(_) => continue,
                };
                size += metadata.len();
            }
        }
    }
    size
}
