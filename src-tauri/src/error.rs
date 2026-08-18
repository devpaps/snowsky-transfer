use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("MTP error: {0}")]
    Mtp(String),

    #[error("Metadata error: {0}")]
    Metadata(String),

    #[error("Converter error: {0}")]
    Converter(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Task error: {0}")]
    Task(String),
}

/// Tauri requires serialisable errors so they can be sent to the JS frontend.
impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
