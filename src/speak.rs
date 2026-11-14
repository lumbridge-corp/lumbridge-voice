//! The text-to-speech server behind `/v1/audio/speech`, reached over HTTP. Today that
//! is NVIDIA Magpie TTS (by default on loopback `:8808`); requests go to it untouched.

use std::time::Duration;

use serde_json::Value;

use crate::adapter::{Failure, Reply, Speak};

pub struct HttpSpeak {
    url: String,
    http: reqwest::Client,
}

impl HttpSpeak {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(3600))
                .build()
                .expect("HTTP client"),
        }
    }

    async fn get_json(&self, path: &str) -> Option<Value> {
        let reply = self
            .http
            .get(format!("{}{path}", self.url))
            .timeout(Duration::from_secs(4))
            .send()
            .await
            .ok()?;
        reply.json().await.ok()
    }
}

impl Speak for HttpSpeak {
    async fn ready(&self) -> bool {
        self.get_json("/ready")
            .await
            .is_some_and(|v| v["ready"] == Value::Bool(true))
    }

    async fn models(&self) -> Vec<Value> {
        self.get_json("/v1/models")
            .await
            .and_then(|v| v["data"].as_array().cloned())
            .unwrap_or_default()
    }

    async fn speech(&self, content_type: &str, body: Vec<u8>) -> Result<Reply, Failure> {
        let request = self
            .http
            .post(format!("{}/v1/audio/speech", self.url))
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body);
        let reply = request
            .send()
            .await
            .map_err(|e| Failure::Unreachable(e.to_string()))?;
        let status = reply.status().as_u16();
        let content_type = reply
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = reply
            .bytes()
            .await
            .map_err(|e| Failure::Unreachable(e.to_string()))?
            .to_vec();
        Ok(Reply {
            status,
            content_type,
            body,
        })
    }
}
