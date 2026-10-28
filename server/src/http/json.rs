//! JSON request extractor + response wrapper.
//!
//! A thin skin over `axum::Json` so a body we cannot parse (wrong
//! `Content-Type`, invalid JSON, missing or mistyped field, bad base64 in
//! a byte field) comes back as our `400 bad_request` error envelope
//! rather than axum's plain-text 415 / 422. Clients already decode
//! `ErrorBody` on every non-2xx, so they get a readable reason for free.

use axum::extract::{FromRequest, Request};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::ApiError;

pub struct Json<T>(pub T);

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let axum::Json(value) = axum::Json::<T>::from_request(req, state)
            .await
            .map_err(|e| ApiError::BadRequest(e.body_text()))?;
        Ok(Json(value))
    }
}

impl<T> IntoResponse for Json<T>
where
    T: Serialize,
{
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}
