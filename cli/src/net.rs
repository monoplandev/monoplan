//! Tiny JSON-over-http client (`spec/auth.md` §Wire encoding). Byte
//! fields are base64 strings courtesy of the protocol types' serde attrs.

use monoplan_protocol::ErrorBody;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

const JSON: &str = "application/json";

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("server returned {status}: {code} — {message}")]
    Api {
        status: StatusCode,
        code: String,
        message: String,
    },
    #[error("request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("encode/decode: {0}")]
    Json(#[from] serde_json::Error),
}

pub struct Client {
    base: String,
    inner: reqwest::Client,
}

impl Client {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            inner: reqwest::Client::new(),
        }
    }

    pub async fn post<Req, Resp>(&self, path: &str, body: &Req) -> Result<Resp, NetError>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        self.send(Method::POST, path, Some(body), None::<&str>)
            .await
    }

    #[allow(dead_code)]
    pub async fn post_authed<Req, Resp>(
        &self,
        path: &str,
        token: &str,
        body: &Req,
    ) -> Result<Resp, NetError>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        self.send(Method::POST, path, Some(body), Some(token)).await
    }

    pub async fn post_authed_no_response<Req>(
        &self,
        path: &str,
        token: &str,
        body: &Req,
    ) -> Result<(), NetError>
    where
        Req: Serialize,
    {
        let _: Empty = self
            .send(Method::POST, path, Some(body), Some(token))
            .await?;
        Ok(())
    }

    /// Unauthenticated GET (the invite payload: the id is the credential).
    pub async fn get_unauthed<Resp>(&self, path: &str) -> Result<Resp, NetError>
    where
        Resp: DeserializeOwned,
    {
        self.send::<(), _>(Method::GET, path, None, None::<&str>)
            .await
    }

    #[allow(dead_code)]
    pub async fn get_authed<Resp>(&self, path: &str, token: &str) -> Result<Resp, NetError>
    where
        Resp: DeserializeOwned,
    {
        self.send::<(), _>(Method::GET, path, None, Some(token))
            .await
    }

    #[allow(dead_code)]
    pub async fn delete_authed(&self, path: &str, token: &str) -> Result<(), NetError> {
        let _: Empty = self
            .send::<(), _>(Method::DELETE, path, None, Some(token))
            .await?;
        Ok(())
    }

    /// `DELETE` carrying a JSON body (`DELETE /api/account`, which
    /// re-verifies the password).
    pub async fn delete_authed_with_body<Req>(
        &self,
        path: &str,
        token: &str,
        body: &Req,
    ) -> Result<(), NetError>
    where
        Req: Serialize,
    {
        let _: Empty = self
            .send(Method::DELETE, path, Some(body), Some(token))
            .await?;
        Ok(())
    }

    /// `PUT` a raw body (an attachment's sealed blob,
    /// `spec/attachments.md` §Routes).
    pub async fn put_bytes(&self, path: &str, token: &str, body: Vec<u8>) -> Result<(), NetError> {
        let resp = self
            .inner
            .put(format!("{}{}", self.base, path))
            .bearer_auth(token)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(body)
            .send()
            .await?;
        Self::raw(resp).await.map(|_| ())
    }

    /// `GET` a raw body.
    pub async fn get_bytes(&self, path: &str, token: &str) -> Result<Vec<u8>, NetError> {
        let resp = self
            .inner
            .get(format!("{}{}", self.base, path))
            .bearer_auth(token)
            .send()
            .await?;
        Self::raw(resp).await
    }

    /// The body of a response that is not JSON, or the server's error.
    async fn raw(resp: reqwest::Response) -> Result<Vec<u8>, NetError> {
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            let err: ErrorBody = serde_json::from_slice(&bytes).unwrap_or(ErrorBody {
                code: "unknown".into(),
                message: String::from_utf8_lossy(&bytes).into_owned(),
            });
            return Err(NetError::Api {
                status,
                code: err.code,
                message: err.message,
            });
        }
        Ok(bytes.to_vec())
    }

    async fn send<Req, Resp>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Req>,
        token: Option<&str>,
    ) -> Result<Resp, NetError>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        let url = format!("{}{}", self.base, path);
        let mut req = self.inner.request(method, url);
        if let Some(b) = body {
            let bytes = serde_json::to_vec(b)?;
            req = req.header(CONTENT_TYPE, JSON).body(bytes);
        }
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            let err: ErrorBody = serde_json::from_slice(&bytes).unwrap_or(ErrorBody {
                code: "unknown".into(),
                message: String::from_utf8_lossy(&bytes).into_owned(),
            });
            return Err(NetError::Api {
                status,
                code: err.code,
                message: err.message,
            });
        }
        // Empty body (HTTP 200 with no payload) is decoded as `Empty`.
        if bytes.is_empty() {
            return Ok(serde_json::from_slice(b"{}")?);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}

/// Stand-in for handlers that return no body. Decodes from `{}` so
/// `serde` is happy.
#[derive(Debug, serde::Deserialize)]
struct Empty {}
