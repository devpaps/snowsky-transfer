//! Read and write audio metadata (tags + cover art) using lofty.

use base64::Engine;
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::probe::Probe;
use std::path::Path;

use crate::error::AppError;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_number: Option<u32>,
    pub duration_ms: u32,
    /// JPEG/PNG as a base64 data-URL, e.g. "data:image/jpeg;base64,..."
    pub cover_art: Option<String>,
}

/// Read all metadata from an audio file, including cover art.
pub fn read(path: &str) -> Result<TrackMetadata, AppError> {
    read_impl(path, true)
}

/// Read metadata from an audio file, skipping cover art.
///
/// Used for bulk scans where the cover is lazy-loaded separately; avoids the
/// allocation + base64 encoding of every embedded picture.
pub fn read_tags(path: &str) -> Result<TrackMetadata, AppError> {
    read_impl(path, false)
}

fn read_impl(path: &str, include_cover: bool) -> Result<TrackMetadata, AppError> {
    let tagged = Probe::open(path)
        .map_err(|e| AppError::Metadata(e.to_string()))?
        .read()
        .map_err(|e| AppError::Metadata(e.to_string()))?;

    let duration_ms = tagged.properties().duration().as_millis() as u32;

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let (title, artist, album, genre, year, track_number, cover_art) = match tag {
        None => (None, None, None, None, None, None, None),
        Some(t) => {
            let cover_art = if include_cover {
                t.pictures()
                    .iter()
                    .find(|p| p.pic_type() == PictureType::CoverFront)
                    .or_else(|| t.pictures().first())
                    .map(|p| {
                        let mime = match p.mime_type() {
                            Some(MimeType::Jpeg) => "image/jpeg",
                            Some(MimeType::Png) => "image/png",
                            _ => "image/jpeg",
                        };
                        let b64 = base64::engine::general_purpose::STANDARD.encode(p.data());
                        format!("data:{mime};base64,{b64}")
                    })
            } else {
                None
            };

            (
                t.title().map(|s| s.to_string()),
                t.artist().map(|s| s.to_string()),
                t.album().map(|s| s.to_string()),
                t.genre().map(|s| s.to_string()),
                t.year(),
                t.track(),
                cover_art,
            )
        }
    };

    Ok(TrackMetadata {
        title,
        artist,
        album,
        genre,
        year,
        track_number,
        duration_ms,
        cover_art,
    })
}

/// Write metadata back to an audio file (modifies in place).
pub fn write(path: &str, meta: &TrackMetadata) -> Result<(), AppError> {
    let mut tagged = Probe::open(path)
        .map_err(|e| AppError::Metadata(e.to_string()))?
        .read()
        .map_err(|e| AppError::Metadata(e.to_string()))?;

    // If the file has no tag, create one appropriate for the format.
    if tagged.first_tag().is_none() {
        let tag_type = tagged.primary_tag_type();
        tagged.insert_tag(lofty::tag::Tag::new(tag_type));
    }

    let tag = match tagged.primary_tag_mut() {
        Some(t) => t,
        None => tagged
            .first_tag_mut()
            .ok_or_else(|| AppError::Metadata("No writable tag found".into()))?,
    };

    if let Some(v) = &meta.title {
        tag.set_title(v.clone());
    }
    if let Some(v) = &meta.artist {
        tag.set_artist(v.clone());
    }
    if let Some(v) = &meta.album {
        tag.set_album(v.clone());
    }
    if let Some(v) = &meta.genre {
        tag.set_genre(v.clone());
    }
    if let Some(v) = meta.year {
        tag.set_year(v);
    }
    if let Some(v) = meta.track_number {
        tag.set_track(v);
    }

    // Cover art: decode the base64 data-URL if provided.
    if let Some(data_url) = &meta.cover_art {
        if let Some(b64) = data_url.strip_prefix("data:image/jpeg;base64,") {
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                let pic = Picture::new_unchecked(
                    PictureType::CoverFront,
                    Some(MimeType::Jpeg),
                    None,
                    bytes,
                );
                tag.set_picture(0, pic);
            }
        } else if let Some(b64) = data_url.strip_prefix("data:image/png;base64,") {
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                let pic = Picture::new_unchecked(
                    PictureType::CoverFront,
                    Some(MimeType::Png),
                    None,
                    bytes,
                );
                tag.set_picture(0, pic);
            }
        }
    }

    tagged
        .save_to_path(Path::new(path), WriteOptions::default())
        .map_err(|e| AppError::Metadata(e.to_string()))?;

    Ok(())
}
