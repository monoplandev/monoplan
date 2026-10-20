//! Tiny msgpack-over-http client.

use monoplan_protocol::ErrorBody;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

const MSGPACK: &str = "application/msgpack";

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
    #[error("encode: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("decode: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
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
            let bytes = rmp_serde::to_vec_named(b)?;
            req = req.header(CONTENT_TYPE, MSGPACK).body(bytes);
        }
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            let err: ErrorBody = rmp_serde::from_slice(&bytes).unwrap_or(ErrorBody {
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
            return Ok(rmp_serde::from_slice(&[0x80][..])?);
        }
        Ok(rmp_serde::from_slice(&bytes)?)
    }
}

/// Stand-in for handlers that return no body. Decodes from an empty
/// fixmap (`0x80`) so `serde` is happy.
#[derive(Debug, serde::Deserialize)]
struct Empty {}
