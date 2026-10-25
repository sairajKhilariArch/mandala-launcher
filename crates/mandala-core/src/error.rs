use thiserror::Error;

pub type Result<T> = std::result::Result<T, MandalaError>;

#[derive(Debug, Error)]
pub enum MandalaError {
    #[error("module `{0}` is not installed")]
    NotInstalled(String),

    #[error("module `{module}` was quarantined: {reason}")]
    Quarantined { module: String, reason: String },

    #[error("module `{module}` failed to load: {reason}")]
    LoadFailed { module: String, reason: String },

    #[error("ACL violation: `{sender}` may not send to `{target}`")]
    AclViolation { sender: String, target: String },

    #[error("IPC channel closed or full")]
    IpcChannel,

    #[error("invalid theme: {0}")]
    Theme(String),

    #[error("crypto error: {0}")]
    Crypto(String),

    #[error("I/O error on {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

impl MandalaError {
    pub fn io(path: impl AsRef<std::path::Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().display().to_string(),
            source,
        }
    }
}
