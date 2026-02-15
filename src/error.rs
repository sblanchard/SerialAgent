use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use serde_json::json;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("HTTP client error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Workspace file not found: {0}")]
    WorkspaceFileNotFound(String),

    #[error("Skill not found: {0}")]
    SkillNotFound(String),

    #[error("SerialMemory error: {0}")]
    SerialMemory(String),

    #[error("Configuration error: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            Error::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            Error::Json(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Error::Toml(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Error::Http(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            Error::WorkspaceFileNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Error::SkillNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Error::SerialMemory(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            Error::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        let body = json!({ "error": message });
        (status, Json(body)).into_response()
    }
}
