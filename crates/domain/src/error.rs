/// Shared error type used across all SerialAgent crates.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP: {0}")]
    Http(String),

    #[error("timeout: {0}")]
    Timeout(String),

    #[error("provider {provider}: {message}")]
    Provider { provider: String, message: String },

    #[error("SerialMemory: {0}")]
    SerialMemory(String),

    #[error("skill not found: {0}")]
    SkillNotFound(String),

    #[error("config: {0}")]
    Config(String),

    #[error("auth: {0}")]
    Auth(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
