use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("too many requests")]
    RateLimited,
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Pool(#[from] r2d2::Error),
    #[error(transparent)]
    DbLayer(#[from] crate::db::DbError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Forbidden => StatusCode::FORBIDDEN,
            AppError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = %self, "request failed");
        }
        (status, status.canonical_reason().unwrap_or("error")).into_response()
    }
}
