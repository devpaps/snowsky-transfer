//! Audio format conversion via the system `ffmpeg` binary.

use std::path::{Path, PathBuf};
use std::process::Command;
use crate::error::AppError;

/// Supported output formats and their ffmpeg codec flags.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConvertRequest {
    pub input_path:  String,
    pub output_fmt:  String,   // "mp3" | "flac" | "ogg" | "m4a" | "aac" | "wav"
    pub bitrate_kbps: Option<u32>, // for lossy formats; None = ffmpeg default
}

/// Convert an audio file and return the path to the temp output file.
/// The caller is responsible for deleting the file when done.
pub fn convert(req: &ConvertRequest) -> Result<String, AppError> {
    // Build output path in system temp dir, same stem, new extension
    let stem = Path::new(&req.input_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("converted");

    let out_dir  = std::env::temp_dir();
    let out_path = unique_path(&out_dir, stem, &req.output_fmt);

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y")                       // overwrite without asking
       .arg("-i").arg(&req.input_path);

    // Codec selection
    match req.output_fmt.as_str() {
        "mp3" => {
            cmd.arg("-codec:a").arg("libmp3lame");
            if let Some(br) = req.bitrate_kbps {
                cmd.arg("-b:a").arg(format!("{br}k"));
            } else {
                cmd.arg("-q:a").arg("2"); // VBR ~190 kbps
            }
        }
        "flac" => {
            cmd.arg("-codec:a").arg("flac");
        }
        "ogg" => {
            cmd.arg("-codec:a").arg("libvorbis");
            if let Some(br) = req.bitrate_kbps {
                cmd.arg("-b:a").arg(format!("{br}k"));
            }
        }
        "m4a" | "aac" => {
            cmd.arg("-codec:a").arg("aac");
            if let Some(br) = req.bitrate_kbps {
                cmd.arg("-b:a").arg(format!("{br}k"));
            }
        }
        "wav" => {
            cmd.arg("-codec:a").arg("pcm_s16le");
        }
        other => {
            return Err(AppError::Converter(format!("Unknown format: {other}")));
        }
    }

    cmd.arg(out_path.to_str().unwrap());

    let status = cmd
        .status()
        .map_err(|e| AppError::Converter(format!("ffmpeg not found or failed to start: {e}")))?;

    if !status.success() {
        return Err(AppError::Converter(format!(
            "ffmpeg exited with code {}",
            status.code().unwrap_or(-1)
        )));
    }

    out_path
        .into_os_string()
        .into_string()
        .map_err(|_| AppError::Converter("Non-UTF8 temp path".into()))
}

/// Check whether ffmpeg is available on PATH.
pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ─── helpers ──────────────────────────────────────────────────────────────────

fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let candidate = dir.join(format!("{stem}.{ext}"));
    if !candidate.exists() {
        return candidate;
    }
    // Append a counter to avoid collisions
    for i in 1..=9999 {
        let p = dir.join(format!("{stem}_{i}.{ext}"));
        if !p.exists() {
            return p;
        }
    }
    dir.join(format!("{stem}_converted.{ext}"))
}
