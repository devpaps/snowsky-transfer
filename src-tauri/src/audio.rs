//! Audio preview — plays a local file in a background thread using rodio.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use crate::error::AppError;

pub struct AudioPlayer {
    stop_flag: Option<Arc<AtomicBool>>,
    handle:    Option<JoinHandle<()>>,
}

impl AudioPlayer {
    pub fn new() -> Self {
        Self { stop_flag: None, handle: None }
    }

    /// Start playing a file. Stops any currently playing audio first.
    pub fn play(&mut self, path: String) -> Result<(), AppError> {
        self.stop();

        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            let (_stream, stream_handle) = match rodio::OutputStream::try_default() {
                Ok(v)  => v,
                Err(e) => { log::error!("Audio output error: {e}"); return; }
            };

            let sink = match rodio::Sink::try_new(&stream_handle) {
                Ok(s)  => s,
                Err(e) => { log::error!("Sink error: {e}"); return; }
            };

            let file = match std::fs::File::open(&path) {
                Ok(f)  => f,
                Err(e) => { log::error!("File error: {e}"); return; }
            };

            let source = match rodio::Decoder::new(std::io::BufReader::new(file)) {
                Ok(s)  => s,
                Err(e) => { log::error!("Decoder error: {e}"); return; }
            };

            sink.append(source);

            // Poll until the sink is empty or a stop signal is received.
            while !sink.empty() && !stop2.load(Ordering::Relaxed) {
                thread::sleep(std::time::Duration::from_millis(100));
            }

            sink.stop();
        });

        self.stop_flag = Some(stop);
        self.handle    = Some(handle);
        Ok(())
    }

    /// Stop playback immediately.
    pub fn stop(&mut self) {
        if let Some(flag) = self.stop_flag.take() {
            flag.store(true, Ordering::Relaxed);
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}
