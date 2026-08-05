//! Safe wrapper around libmtp for communicating with the Snowsky Echo Mini.
//!
//! Struct layouts are hand-written to match libmtp 1.1.x on 64-bit Linux.
//! If you run into crashes, generate bindings instead:
//!   cargo add bindgen --build && add bindgen::Builder in build.rs

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::Arc;
use tauri::Emitter;

use crate::error::AppError;

// ─── FFI declarations ────────────────────────────────────────────────────────

#[allow(non_camel_case_types, non_snake_case, dead_code)]
mod ffi {
    use std::os::raw::{c_char, c_int, c_uint, c_void};

    // LIBMTP_filetype_t enum values (sequential, starting at 0)
    pub type LIBMTP_filetype_t = c_uint;
    pub const LIBMTP_FILETYPE_WAV:     LIBMTP_filetype_t = 0;
    pub const LIBMTP_FILETYPE_MP3:     LIBMTP_filetype_t = 1;
    pub const LIBMTP_FILETYPE_OGG:     LIBMTP_filetype_t = 3;
    pub const LIBMTP_FILETYPE_MP4:     LIBMTP_filetype_t = 5;
    pub const LIBMTP_FILETYPE_AAC:     LIBMTP_filetype_t = 29;
    pub const LIBMTP_FILETYPE_FLAC:    LIBMTP_filetype_t = 31;
    pub const LIBMTP_FILETYPE_M4A:     LIBMTP_filetype_t = 33;
    pub const LIBMTP_FILETYPE_UNKNOWN: LIBMTP_filetype_t = 44;

    pub type LIBMTP_error_number_t = c_uint;
    pub const LIBMTP_ERROR_NONE: LIBMTP_error_number_t = 0;

    #[repr(C)]
    pub struct LIBMTP_error_t {
        pub errornumber: LIBMTP_error_number_t,
        pub error_text: *mut c_char,
        pub next: *mut LIBMTP_error_t,
    }

    pub type LIBMTP_progressfunc_t =
        Option<unsafe extern "C" fn(u64, u64, *const c_void) -> c_int>;

    // Opaque device handle
    pub enum LIBMTP_mtpdevice_t {}

    #[repr(C)]
    pub struct LIBMTP_filesampledata_t {
        pub width:    u32,
        pub height:   u32,
        pub duration: u32,
        pub filetype: LIBMTP_filetype_t,
        pub size:     u64,
        pub data:     *mut c_char,
    }

    /// Hand-matched to libmtp 1.1.x on x86_64 Linux.
    /// Layout verified against libmtp.h – total size 144 bytes.
    #[repr(C)]
    pub struct LIBMTP_track_t {
        pub item_id:          u32,             // +0
        pub parent_id:        u32,             // +4
        pub storage_id:       u32,             // +8
        _pad0:                u32,             // +12  (alignment for *mut c_char)
        pub title:            *mut c_char,     // +16
        pub artist:           *mut c_char,     // +24
        pub composer:         *mut c_char,     // +32
        pub genre:            *mut c_char,     // +40
        pub album:            *mut c_char,     // +48
        pub date:             *mut c_char,     // +56
        pub filename:         *mut c_char,     // +64
        pub tracknumber:      u16,             // +72
        _pad1:                u16,             // +74
        pub duration:         u32,             // +76
        pub samplerate:       u32,             // +80
        pub nochannels:       u16,             // +84
        _pad2:                u16,             // +86
        pub wavecodec:        u32,             // +88
        pub bitrate:          u32,             // +92
        pub bitratetype:      u16,             // +96
        pub rating:           u16,             // +98
        pub usecount:         u32,             // +100
        pub filesize:         u64,             // +104  (naturally 8-aligned)
        pub modificationdate: i64,             // +112  (time_t on 64-bit Linux)
        pub filetype:         LIBMTP_filetype_t, // +120
        _pad3:                u32,             // +124
        pub sampledata:       *mut LIBMTP_filesampledata_t, // +128
        pub next:             *mut LIBMTP_track_t,          // +136
    }                                          // total = 144

    #[repr(C)]
    pub struct LIBMTP_playlist_t {
        pub playlist_id: u32,
        pub parent_id:   u32,
        pub storage_id:  u32,
        _pad:            u32,
        pub name:        *mut c_char,
        pub tracks:      *mut u32,
        pub no_tracks:   u32,
        _pad2:           u32,
        pub next:        *mut LIBMTP_playlist_t,
    }

    #[repr(C)]
    pub struct LIBMTP_device_entry_t {
        pub vendor:       *mut c_char,
        pub vendor_id:    u16,
        pub product:      *mut c_char,
        pub product_id:   u16,
        pub device_flags: u32,
    }

    #[repr(C)]
    pub struct LIBMTP_raw_device_t {
        pub device_entry: LIBMTP_device_entry_t,
        pub bus_location: u32,
        pub devnum:       u8,
    }

    extern "C" {
        pub fn LIBMTP_Init();
        pub fn LIBMTP_Detect_Raw_Devices(
            devices:       *mut *mut LIBMTP_raw_device_t,
            numrawdevices: *mut c_int,
        ) -> LIBMTP_error_number_t;
        pub fn LIBMTP_Open_Raw_Device_Uncached(
            rawdevice: *mut LIBMTP_raw_device_t,
        ) -> *mut LIBMTP_mtpdevice_t;
        pub fn LIBMTP_Release_Device(device: *mut LIBMTP_mtpdevice_t);

        pub fn LIBMTP_Get_Tracklisting_With_Callback(
            device:   *mut LIBMTP_mtpdevice_t,
            callback: LIBMTP_progressfunc_t,
            data:     *const c_void,
        ) -> *mut LIBMTP_track_t;

        pub fn LIBMTP_Send_Track_From_File(
            device:   *mut LIBMTP_mtpdevice_t,
            path:     *const c_char,
            metadata: *mut LIBMTP_track_t,
            callback: LIBMTP_progressfunc_t,
            data:     *const c_void,
        ) -> c_int;

        pub fn LIBMTP_Delete_Object(device: *mut LIBMTP_mtpdevice_t, object_id: u32) -> c_int;

        pub fn LIBMTP_Get_Playlist_List(
            device: *mut LIBMTP_mtpdevice_t,
        ) -> *mut LIBMTP_playlist_t;
        pub fn LIBMTP_Create_New_Playlist(
            device:   *mut LIBMTP_mtpdevice_t,
            metadata: *mut LIBMTP_playlist_t,
        ) -> c_int;
        pub fn LIBMTP_Update_Playlist(
            device:   *mut LIBMTP_mtpdevice_t,
            metadata: *mut LIBMTP_playlist_t,
        ) -> c_int;
        pub fn LIBMTP_destroy_playlist_t(playlist: *mut LIBMTP_playlist_t);
        pub fn LIBMTP_destroy_track_t(track: *mut LIBMTP_track_t);

        pub fn LIBMTP_Dump_Errorstack(device: *mut LIBMTP_mtpdevice_t);
        pub fn LIBMTP_Get_Errorstack(device: *mut LIBMTP_mtpdevice_t) -> *mut LIBMTP_error_t;
        pub fn LIBMTP_Clear_Errorstack(device: *mut LIBMTP_mtpdevice_t);

        pub fn LIBMTP_Get_Friendlyname(device: *mut LIBMTP_mtpdevice_t) -> *mut c_char;
        pub fn LIBMTP_Get_Modelname(device:   *mut LIBMTP_mtpdevice_t) -> *mut c_char;
        pub fn LIBMTP_Get_Serialnumber(device: *mut LIBMTP_mtpdevice_t) -> *mut c_char;
        pub fn LIBMTP_Get_Batterylevel(
            device:        *mut LIBMTP_mtpdevice_t,
            maximum_level: *mut u8,
            current_level: *mut u8,
        ) -> c_int;
    }
}

// ─── Helper: C-string → owned String ─────────────────────────────────────────

/// Copy a nullable C-string to an owned Rust String without freeing the pointer.
/// (Field strings inside LIBMTP_track_t are owned by the struct.)
unsafe fn ptr_to_string(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// Copy a C-string and then free the pointer (for strings returned by libmtp getters).
unsafe fn take_string(p: *mut c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = CStr::from_ptr(p).to_string_lossy().into_owned();
    libc::free(p as *mut c_void);
    s
}

// ─── filetype helpers ─────────────────────────────────────────────────────────

fn ext_to_filetype(path: &str) -> ffi::LIBMTP_filetype_t {
    match std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("mp3")  => ffi::LIBMTP_FILETYPE_MP3,
        Some("flac") => ffi::LIBMTP_FILETYPE_FLAC,
        Some("ogg")  => ffi::LIBMTP_FILETYPE_OGG,
        Some("wav")  => ffi::LIBMTP_FILETYPE_WAV,
        Some("m4a")  => ffi::LIBMTP_FILETYPE_M4A,
        Some("aac")  => ffi::LIBMTP_FILETYPE_AAC,
        Some("mp4")  => ffi::LIBMTP_FILETYPE_MP4,
        _            => ffi::LIBMTP_FILETYPE_UNKNOWN,
    }
}

fn filetype_to_str(ft: ffi::LIBMTP_filetype_t) -> String {
    match ft {
        ffi::LIBMTP_FILETYPE_MP3  => "MP3",
        ffi::LIBMTP_FILETYPE_FLAC => "FLAC",
        ffi::LIBMTP_FILETYPE_OGG  => "OGG",
        ffi::LIBMTP_FILETYPE_WAV  => "WAV",
        ffi::LIBMTP_FILETYPE_M4A  => "M4A",
        ffi::LIBMTP_FILETYPE_AAC  => "AAC",
        ffi::LIBMTP_FILETYPE_MP4  => "MP4",
        _                         => "Unknown",
    }
    .to_string()
}

fn take_error_stack(ptr: *mut ffi::LIBMTP_mtpdevice_t) -> String {
    let mut messages = Vec::new();
    unsafe {
        let mut error = ffi::LIBMTP_Get_Errorstack(ptr);
        while !error.is_null() {
            if !(*error).error_text.is_null() {
                messages.push(CStr::from_ptr((*error).error_text).to_string_lossy().into_owned());
            }
            error = (*error).next;
        }
        ffi::LIBMTP_Clear_Errorstack(ptr);
    }
    messages.join("; ")
}

// ─── Public types (serialised to JSON for the frontend) ──────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceInfo {
    pub name:          String,
    pub model:         String,
    pub serial:        String,
    pub battery_level: Option<u8>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Track {
    pub id:           u32,
    pub title:        String,
    pub artist:       String,
    pub album:        String,
    pub genre:        String,
    pub filename:     String,
    pub duration_ms:  u32,
    pub filesize:     u64,
    pub filetype:     String,
    pub track_number: u16,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Playlist {
    pub id:        u32,
    pub name:      String,
    pub track_ids: Vec<u32>,
}

/// Request sent from the JS side when transferring a file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SendTrackRequest {
    pub path:         String,
    pub filename:     Option<String>,
    pub title:        Option<String>,
    pub artist:       Option<String>,
    pub album:        Option<String>,
    pub genre:        Option<String>,
    pub track_number: Option<u16>,
    pub duration_ms:  Option<u32>,
}

// ─── Progress callback ───────────────────────────────────────────────────────

struct ProgressCtx {
    app_handle:  Arc<tauri::AppHandle>,
    transfer_id: String,
}

unsafe extern "C" fn libmtp_progress(
    sent:  u64,
    total: u64,
    data:  *const c_void,
) -> c_int {
    if !data.is_null() {
        let ctx = &*(data as *const ProgressCtx);
        let percent = if total > 0 { sent * 100 / total } else { 0 };
        let _ = ctx.app_handle.emit(
            "transfer:progress",
            serde_json::json!({
                "id":      ctx.transfer_id,
                "sent":    sent,
                "total":   total,
                "percent": percent,
            }),
        );
    }
    0 // returning non-zero would cancel the transfer
}

// ─── Device wrapper ───────────────────────────────────────────────────────────

struct RawDevice(*mut ffi::LIBMTP_mtpdevice_t);

// SAFETY: We only access the device handle while holding a Mutex, so single-
// threaded access is guaranteed. libmtp itself is not thread-safe.
unsafe impl Send for RawDevice {}

// ─── MtpManager ──────────────────────────────────────────────────────────────

pub struct MtpManager {
    device: Option<RawDevice>,
}

impl MtpManager {
    pub fn new() -> Self {
        unsafe { ffi::LIBMTP_Init() };
        Self { device: None }
    }

    fn device_ptr(&mut self) -> Result<*mut ffi::LIBMTP_mtpdevice_t, AppError> {
        self.device
            .as_ref()
            .map(|d| d.0)
            .ok_or_else(|| AppError::Mtp("No device connected".into()))
    }

    /// Scan for connected MTP devices and open the first one found.
    pub fn scan(&mut self) -> Result<Option<DeviceInfo>, AppError> {
        self.disconnect();

        unsafe {
            let mut raw_list: *mut ffi::LIBMTP_raw_device_t = std::ptr::null_mut();
            let mut count: c_int = 0;

            let err = ffi::LIBMTP_Detect_Raw_Devices(&mut raw_list, &mut count);

            if err != ffi::LIBMTP_ERROR_NONE || count == 0 {
                if !raw_list.is_null() {
                    libc::free(raw_list as *mut c_void);
                }
                return Ok(None);
            }

            let dev_ptr = ffi::LIBMTP_Open_Raw_Device_Uncached(raw_list);
            libc::free(raw_list as *mut c_void);

            if dev_ptr.is_null() {
                return Err(AppError::Mtp("Failed to open device".into()));
            }

            let name    = take_string(ffi::LIBMTP_Get_Friendlyname(dev_ptr));
            let model   = take_string(ffi::LIBMTP_Get_Modelname(dev_ptr));
            let serial  = take_string(ffi::LIBMTP_Get_Serialnumber(dev_ptr));

            let battery_level = {
                let (mut max, mut cur) = (0u8, 0u8);
                if ffi::LIBMTP_Get_Batterylevel(dev_ptr, &mut max, &mut cur) == 0 && max > 0 {
                    Some((cur as u32 * 100 / max as u32) as u8)
                } else {
                    None
                }
            };

            self.device = Some(RawDevice(dev_ptr));

            Ok(Some(DeviceInfo {
                name: if name.is_empty() { model.clone() } else { name },
                model,
                serial,
                battery_level,
            }))
        }
    }

    pub fn disconnect(&mut self) {
        if let Some(RawDevice(ptr)) = self.device.take() {
            unsafe { ffi::LIBMTP_Release_Device(ptr) };
        }
    }

    pub fn is_connected(&self) -> bool {
        self.device.is_some()
    }

    /// List all tracks currently on the device.
    pub fn get_tracks(&mut self) -> Result<Vec<Track>, AppError> {
        let ptr = self.device_ptr()?;
        let mut tracks = Vec::new();

        unsafe {
            let list = ffi::LIBMTP_Get_Tracklisting_With_Callback(ptr, None, std::ptr::null());

            let mut cur = list;
            while !cur.is_null() {
                let next = (*cur).next;
                let t = &*cur;

                tracks.push(Track {
                    id:           t.item_id,
                    title:        ptr_to_string(t.title),
                    artist:       ptr_to_string(t.artist),
                    album:        ptr_to_string(t.album),
                    genre:        ptr_to_string(t.genre),
                    filename:     ptr_to_string(t.filename),
                    duration_ms:  t.duration,
                    filesize:     t.filesize,
                    filetype:     filetype_to_str(t.filetype),
                    track_number: t.tracknumber,
                });

                // Free this node (not its next pointer — we saved that above)
                (*cur).next = std::ptr::null_mut();
                ffi::LIBMTP_destroy_track_t(cur);
                cur = next;
            }

            // Dump + clear any non-fatal errors (e.g. unsupported track types)
            ffi::LIBMTP_Dump_Errorstack(ptr);
            ffi::LIBMTP_Clear_Errorstack(ptr);
        }

        Ok(tracks)
    }

    /// Transfer a single audio file to the device.
    pub fn send_track(
        &mut self,
        req:         &SendTrackRequest,
        app_handle:  Arc<tauri::AppHandle>,
        transfer_id: &str,
    ) -> Result<u32, AppError> {
        let ptr = self.device_ptr()?;

        let path_c = CString::new(req.path.as_str())
            .map_err(|e| AppError::Mtp(e.to_string()))?;

        let filename = req.filename.clone().unwrap_or_else(|| {
            std::path::Path::new(&req.path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("track")
                .to_string()
        });

        let title_c    = CString::new(req.title.as_deref().unwrap_or(&filename)).unwrap();
        let artist_c   = CString::new(req.artist.as_deref().unwrap_or("")).unwrap();
        let album_c    = CString::new(req.album.as_deref().unwrap_or("")).unwrap();
        let genre_c    = CString::new(req.genre.as_deref().unwrap_or("")).unwrap();
        let filename_c = CString::new(filename.as_str()).unwrap();

        let filesize = std::fs::metadata(&req.path)?.len();
        let filetype = ext_to_filetype(&req.path);

        let ctx = ProgressCtx {
            app_handle,
            transfer_id: transfer_id.to_string(),
        };

        let mut meta: ffi::LIBMTP_track_t = unsafe { std::mem::zeroed() };
        meta.parent_id  = 0xFFFF_FFFF;
        meta.title      = title_c.as_ptr() as *mut c_char;
        meta.artist     = artist_c.as_ptr() as *mut c_char;
        meta.genre      = genre_c.as_ptr() as *mut c_char;
        meta.album      = album_c.as_ptr() as *mut c_char;
        meta.filename   = filename_c.as_ptr() as *mut c_char;
        meta.tracknumber = req.track_number.unwrap_or(0);
        meta.duration   = req.duration_ms.unwrap_or(0);
        meta.filesize   = filesize;
        meta.filetype   = filetype;

        let ret = unsafe {
            ffi::LIBMTP_Send_Track_From_File(
                ptr,
                path_c.as_ptr(),
                &mut meta,
                Some(libmtp_progress),
                &ctx as *const ProgressCtx as *const c_void,
            )
        };

        if ret != 0 {
            let detail = take_error_stack(ptr);
            return Err(AppError::Mtp(format!(
                "Transfer failed for \"{}\"{}",
                filename,
                if detail.is_empty() { String::new() } else { format!(": {detail}") },
            )));
        }

        Ok(meta.item_id)
    }

    /// Delete a track from the device by its MTP object ID.
    pub fn delete_track(&mut self, track_id: u32) -> Result<(), AppError> {
        let ptr = self.device_ptr()?;
        let ret = unsafe { ffi::LIBMTP_Delete_Object(ptr, track_id) };
        if ret != 0 {
            unsafe {
                ffi::LIBMTP_Dump_Errorstack(ptr);
                ffi::LIBMTP_Clear_Errorstack(ptr);
            }
            return Err(AppError::Mtp(format!("Failed to delete track {track_id}")));
        }
        Ok(())
    }

    /// List all playlists on the device.
    pub fn get_playlists(&mut self) -> Result<Vec<Playlist>, AppError> {
        let ptr = self.device_ptr()?;
        let mut playlists = Vec::new();

        unsafe {
            let list = ffi::LIBMTP_Get_Playlist_List(ptr);
            let mut cur = list;
            while !cur.is_null() {
                let next = (*cur).next;
                let p = &*cur;

                let mut track_ids = Vec::new();
                for i in 0..p.no_tracks as usize {
                    track_ids.push(*p.tracks.add(i));
                }

                playlists.push(Playlist {
                    id:        p.playlist_id,
                    name:      ptr_to_string(p.name),
                    track_ids,
                });

                (*cur).next = std::ptr::null_mut();
                ffi::LIBMTP_destroy_playlist_t(cur);
                cur = next;
            }
        }

        Ok(playlists)
    }

    /// Create a new playlist on the device.
    pub fn create_playlist(
        &mut self,
        name:      &str,
        track_ids: &[u32],
    ) -> Result<u32, AppError> {
        let ptr = self.device_ptr()?;
        let name_c = CString::new(name).map_err(|e| AppError::Mtp(e.to_string()))?;

        let mut ids_copy = track_ids.to_vec();
        let tracks_ptr = if ids_copy.is_empty() {
            std::ptr::null_mut()
        } else {
            ids_copy.as_mut_ptr()
        };

        let mut meta: ffi::LIBMTP_playlist_t = unsafe { std::mem::zeroed() };
        meta.parent_id = 0xFFFF_FFFF;
        meta.name      = name_c.as_ptr() as *mut c_char;
        meta.tracks    = tracks_ptr;
        meta.no_tracks = ids_copy.len() as u32;

        let ret = unsafe { ffi::LIBMTP_Create_New_Playlist(ptr, &mut meta) };

        if ret != 0 {
            unsafe {
                ffi::LIBMTP_Dump_Errorstack(ptr);
                ffi::LIBMTP_Clear_Errorstack(ptr);
            }
            return Err(AppError::Mtp(format!("Failed to create playlist \"{name}\"")));
        }

        Ok(meta.playlist_id)
    }

    /// Update an existing playlist's track list.
    pub fn update_playlist(
        &mut self,
        playlist_id: u32,
        name:        &str,
        track_ids:   &[u32],
    ) -> Result<(), AppError> {
        let ptr = self.device_ptr()?;
        let name_c = CString::new(name).map_err(|e| AppError::Mtp(e.to_string()))?;
        let mut ids_copy = track_ids.to_vec();

        let mut meta: ffi::LIBMTP_playlist_t = unsafe { std::mem::zeroed() };
        meta.parent_id = 0xFFFF_FFFF;
        meta.playlist_id = playlist_id;
        meta.name      = name_c.as_ptr() as *mut c_char;
        meta.tracks    = if ids_copy.is_empty() { std::ptr::null_mut() } else { ids_copy.as_mut_ptr() };
        meta.no_tracks = ids_copy.len() as u32;

        let ret = unsafe { ffi::LIBMTP_Update_Playlist(ptr, &mut meta) };
        if ret != 0 {
            unsafe {
                ffi::LIBMTP_Dump_Errorstack(ptr);
                ffi::LIBMTP_Clear_Errorstack(ptr);
            }
            return Err(AppError::Mtp(format!("Failed to update playlist {playlist_id}")));
        }
        Ok(())
    }

    /// Delete a playlist (the tracks themselves remain on the device).
    pub fn delete_playlist(&mut self, playlist_id: u32) -> Result<(), AppError> {
        // MTP playlists are just objects; delete by object ID
        self.delete_track(playlist_id)
    }
}

impl Drop for MtpManager {
    fn drop(&mut self) {
        self.disconnect();
    }
}
