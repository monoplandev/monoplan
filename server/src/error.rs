use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use monoplan_protocol::ErrorBody;
use rusqlite::ffi::ErrorCode;

use crate::build_info;
use crate::http::msgpack::Msgpack;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("malformed request: {0}")]
    BadRequest(String),

    #[error("invalid credentials")]
    InvalidCredentials,

    #[error("account exists")]
    AccountExists,

    #[error("not found")]
    NotFound,

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("recovery not enrolled")]
    RecoveryNotEnrolled,

    #[error("recovery session invalid or expired")]
    RecoverySessionInvalid,

    #[error("missing or invalid bearer token")]
    Unauthorized,

    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl ApiError {
    fn status_and_code(&self) -> (StatusCode, &'static str) {
        use ApiError::*;
        match self {
            BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid_credentials"),
            AccountExists => (StatusCode::CONFLICT, "account_exists"),
            NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            RecoveryNotEnrolled => (StatusCode::FORBIDDEN, "recovery_not_enrolled"),
            RecoverySessionInvalid => (StatusCode::FORBIDDEN, "recovery_session_invalid"),
            Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();
        if let ApiError::Internal(err) = &self {
            log_internal_error(err);
        }
        let body = ErrorBody {
            code: code.to_string(),
            message: self.to_string(),
        };
        (status, Msgpack(body)).into_response()
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError::Internal(anyhow::Error::new(e))
    }
}

impl From<tokio_rusqlite::Error> for ApiError {
    fn from(e: tokio_rusqlite::Error) -> Self {
        ApiError::Internal(anyhow::Error::new(e))
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

fn log_internal_error(err: &anyhow::Error) {
    let (kind, code) = classify_internal_error(err);
    tracing::error!(
        build.git_sha = build_info::GIT_SHA,
        error.kind = kind,
        error.code = code,
        error.message = %err,
        error.root_cause = %err.root_cause(),
        error.chain = %format_error_chain(err),
        "internal error"
    );
}

fn format_error_chain(err: &anyhow::Error) -> String {
    err.chain()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

fn classify_internal_error(err: &anyhow::Error) -> (&'static str, &'static str) {
    for cause in err.chain() {
        if let Some(sqlite) = cause.downcast_ref::<rusqlite::Error>() {
            return classify_rusqlite_error(sqlite);
        }
    }
    ("internal", "internal")
}

fn classify_rusqlite_error(err: &rusqlite::Error) -> (&'static str, &'static str) {
    match err {
        rusqlite::Error::SqliteFailure(inner, _) => match inner.code {
            ErrorCode::ConstraintViolation => match inner.extended_code {
                787 => ("db", "sqlite.foreign_key"),
                1555 | 2067 => ("db", "sqlite.unique"),
                _ => ("db", "sqlite.constraint"),
            },
            ErrorCode::DatabaseBusy => ("db", "sqlite.busy"),
            ErrorCode::DatabaseLocked => ("db", "sqlite.locked"),
            _ => ("db", "sqlite.error"),
        },
        _ => ("db", "sqlite.error"),
    }
}
