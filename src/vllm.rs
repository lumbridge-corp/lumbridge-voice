//! Adapter for NVIDIA Nemotron 3.5 Transcribe served by vLLM with NeMo Speech
//! (by default on loopback `:8807`; see `examples/serve-transcribe-vllm.sh`).
//!
//! The model is asked through vLLM's chat API: an instruction from the model card plus
//! the audio as a `data:` URL. Word times and speaker turns come back in a top-level
//! `ctc_timestamps` object when `mm_processor_kwargs.capture_ctc_timestamps` is set.

use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};

use crate::adapter::{Audio, Failure, Reply, Task, Transcriber, Transcript, Turn, Word};
use crate::text::{data_url, spk_int};

// Instructions from the model card's "Prompt Recommendations" table.
const P_CLEAN: &str = "Audio transcription. Language: as spoken. Wording: omit fillers and accidental repetitions; keep the \
remaining spoken wording unchanged. Formatting: punctuation and capitalization. Output: transcript only.";
const P_VERBATIM: &str = "Audio transcription. Language: as spoken. Wording: verbatim, including fillers and repetitions. \
Formatting: punctuation and capitalization. Output: transcript only.";
const P_SPEAKERS: &str = "Transcribe this audio with punctuation and capitalization. Include all spoken words and verbal fillers. \
Write numbers in their standard written form. Put speaker tags <spk:0> <spk:1> <spk:2> <spk:3> on the transcription.";

/// The instruction for a task.
pub fn prompt(task: &Task) -> String {
    match task {
        Task::Verbatim => P_VERBATIM.to_string(),
        Task::Clean => P_CLEAN.to_string(),
        Task::Speakers => P_SPEAKERS.to_string(),
        Task::Context(terms) => {
            format!("Transcribe with punctuation and capitalization. Context terms: {terms}.")
        }
        Task::Translate(language) => format!("Please translate the given speech to {language}."),
    }
}

pub struct Vllm {
    url: String,
    model: String,
    http: reqwest::Client,
}

impl Vllm {
    pub fn new(url: &str, model: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            // The model is on this machine or this network: never through a proxy.
            // An hour per request: recordings can be long.
            http: reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(3600))
                .build()
                .expect("HTTP client"),
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Reply, Failure> {
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

/// Reads the model's answer to a transcription request.
pub fn parse_transcript(answer: &Value) -> Transcript {
    let text = answer["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let ctc = &answer["ctc_timestamps"];
    let seconds = |v: &Value| {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(0.0)
    };
    let list = |key: &str| ctc[key].as_array().cloned().unwrap_or_default();
    Transcript {
        text,
        words: list("words")
            .iter()
            .map(|w| Word {
                word: w["word"].as_str().unwrap_or("").to_string(),
                start: seconds(&w["start"]),
                end: seconds(&w["end"]),
                speaker: spk_int(&w["speaker"]),
            })
            .collect(),
        turns: list("diarization")
            .iter()
            .map(|s| Turn {
                start: seconds(&s["start"]),
                end: seconds(&s["end"]),
                speaker: spk_int(&s["speaker"]),
            })
            .collect(),
        align_error: match &ctc["error"] {
            Value::Null => None,
            Value::String(s) if s.is_empty() => None,
            Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        },
    }
}

/// Rewrites a dictation client's chat request into what the model takes: `input_audio`
/// parts become `audio_url` parts, the model name is ours, and the model card's
/// defaults are filled in where the caller set nothing.
pub fn rewrite_chat(mut body: Value, model: &str) -> Result<Value, Failure> {
    let bad = |what: &str| Failure::BadRequest(what.to_string());
    let request = body
        .as_object_mut()
        .ok_or_else(|| bad("the request body must be a JSON object"))?;
    if let Some(messages) = request.get_mut("messages").and_then(Value::as_array_mut) {
        for part in messages
            .iter_mut()
            .filter_map(|m| m.get_mut("content")?.as_array_mut())
            .flatten()
        {
            if part.get("type").and_then(Value::as_str) != Some("input_audio") {
                continue;
            }
            let Some(part) = part.as_object_mut() else {
                continue;
            };
            let audio = part.remove("input_audio").unwrap_or(Value::Null);
            let data = audio["data"]
                .as_str()
                .ok_or_else(|| bad("input_audio.data must be base64 text"))?;
            let raw = base64::engine::general_purpose::STANDARD
                .decode(data.trim())
                .map_err(|_| bad("input_audio.data is not valid base64"))?;
            let format = audio["format"].as_str().unwrap_or("wav");
            part.insert("type".into(), json!("audio_url"));
            part.insert("audio_url".into(), json!({"url": data_url(&raw, format)}));
        }
    }
    request.insert("model".into(), json!(model));
    request.entry("temperature").or_insert(json!(0));
    let kwargs = request.entry("chat_template_kwargs").or_insert(json!({}));
    if let Some(kwargs) = kwargs.as_object_mut() {
        kwargs.entry("enable_thinking").or_insert(json!(false));
    }
    request.entry("skip_special_tokens").or_insert(json!(false));
    request
        .entry("spaces_between_special_tokens")
        .or_insert(json!(false));
    Ok(body)
}

impl Transcriber for Vllm {
    fn model(&self) -> &str {
        &self.model
    }

    async fn healthy(&self) -> bool {
        let request = self
            .http
            .get(format!("{}/health", self.url))
            .timeout(Duration::from_secs(4));
        matches!(request.send().await, Ok(reply) if reply.status().as_u16() == 200)
    }

    async fn transcribe(
        &self,
        audio: &Audio,
        task: &Task,
        timestamps: bool,
    ) -> Result<Transcript, Failure> {
        let mut body = json!({
            "model": self.model, "temperature": 0, "max_tokens": 16384, "seed": 0,
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": prompt(task)},
                {"type": "audio_url", "audio_url": {"url": data_url(&audio.bytes, &audio.format)}},
            ]}],
            "chat_template_kwargs": {"enable_thinking": false},
            "skip_special_tokens": false, "spaces_between_special_tokens": false,
        });
        if timestamps {
            body["mm_processor_kwargs"] = json!({"capture_ctc_timestamps": true});
        }
        let reply = self
            .send(
                self.http
                    .post(format!("{}/v1/chat/completions", self.url))
                    .json(&body),
            )
            .await?;
        if reply.status != 200 {
            return Err(Failure::Status {
                status: reply.status,
                text: String::from_utf8_lossy(&reply.body).into_owned(),
            });
        }
        let answer: Value = serde_json::from_slice(&reply.body).map_err(|e| Failure::Status {
            status: 502,
            text: format!("the model's answer is not JSON: {e}"),
        })?;
        Ok(parse_transcript(&answer))
    }

    async fn chat(&self, body: Value) -> Result<Reply, Failure> {
        let body = rewrite_chat(body, &self.model)?;
        self.send(
            self.http
                .post(format!("{}/v1/chat/completions", self.url))
                .json(&body),
        )
        .await
    }

    async fn forward(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<Reply, Failure> {
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| Failure::BadRequest("bad method".into()))?;
        let request = self
            .http
            .request(method, format!("{}{}", self.url, path))
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body);
        self.send(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_are_the_model_cards() {
        // Copied from front-py/speech_front.py.
        assert_eq!(prompt(&Task::Clean), "Audio transcription. Language: as spoken. Wording: omit fillers and accidental repetitions; keep the remaining spoken wording unchanged. Formatting: punctuation and capitalization. Output: transcript only.");
        assert_eq!(prompt(&Task::Verbatim), "Audio transcription. Language: as spoken. Wording: verbatim, including fillers and repetitions. Formatting: punctuation and capitalization. Output: transcript only.");
        assert_eq!(prompt(&Task::Speakers), "Transcribe this audio with punctuation and capitalization. Include all spoken words and verbal fillers. Write numbers in their standard written form. Put speaker tags <spk:0> <spk:1> <spk:2> <spk:3> on the transcription.");
        assert_eq!(
            prompt(&Task::Context("Ada, Lumbridge".into())),
            "Transcribe with punctuation and capitalization. Context terms: Ada, Lumbridge."
        );
        assert_eq!(
            prompt(&Task::Translate("German".into())),
            "Please translate the given speech to German."
        );
    }

    #[test]
    fn chat_defaults_do_not_override_the_caller() {
        let asked = json!({"model": "salm", "temperature": 0.4, "chat_template_kwargs": {"enable_thinking": true, "x": 1},
                           "skip_special_tokens": true, "max_tokens": 50, "messages": "not a list"});
        let sent = rewrite_chat(asked, "the-model").unwrap();
        assert_eq!(
            sent,
            json!({"model": "the-model", "temperature": 0.4, "chat_template_kwargs": {"enable_thinking": true, "x": 1},
                                "skip_special_tokens": true, "max_tokens": 50, "messages": "not a list",
                                "spaces_between_special_tokens": false})
        );
        assert!(matches!(
            rewrite_chat(json!([1]), "m"),
            Err(Failure::BadRequest(_))
        ));
        let bad_audio = json!({"messages": [{"role": "user", "content": [{"type": "input_audio", "input_audio": {"data": "***"}}]}]});
        assert!(matches!(
            rewrite_chat(bad_audio, "m"),
            Err(Failure::BadRequest(_))
        ));
    }
}
