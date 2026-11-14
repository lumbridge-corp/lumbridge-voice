//! The HTTP API. Routes, fields, status codes and response shapes are those of the
//! Python front (`docs/API.md`), down to the order of the keys.

// Handlers return an axum Response as their error: it is the answer, sent as is.
#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::multipart::MultipartRejection;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use futures_util::future::try_join_all;
use serde_json::{json, Value};

use crate::adapter::{Audio, Failure, Speak, Task, Transcriber, Turn, Word};
use crate::text::{clean, round3, speaker_at};
use crate::wav;

/// Everything a request needs.
pub struct App<T, S> {
    pub transcriber: T,
    pub speak: S,
    pub ffmpeg: String,
    pub max_upload_bytes: usize,
}

pub fn router<T: Transcriber, S: Speak>(app: Arc<App<T, S>>) -> Router {
    let limit = app.max_upload_bytes;
    Router::new()
        .route("/ready", get(ready::<T, S>))
        .route("/health", get(ready::<T, S>))
        .route("/v1/models", get(models::<T, S>))
        .route("/v1/chat/completions", post(chat::<T, S>))
        .route("/v1/audio/transcriptions", post(transcriptions::<T, S>))
        .route("/v1/audio/diarizations", post(diarizations::<T, S>))
        .route("/v1/audio/translations", post(translations::<T, S>))
        .route("/v1/audio/speech", post(speech::<T, S>))
        // Anything else, and a known path with the other method, goes to the model
        // server, as in the Python front.
        .fallback(forward::<T, S>)
        .method_not_allowed_fallback(forward::<T, S>)
        .layer(DefaultBodyLimit::max(limit))
        .with_state(app)
}

// ── Responses ────────────────────────────────────────────────────────────────

fn raw(status: u16, content_type: Option<&str>, body: Vec<u8>) -> Response {
    let mut response =
        Response::builder().status(StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY));
    if let Some(content_type) = content_type {
        response = response.header(header::CONTENT_TYPE, content_type);
    }
    response
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn json_reply(status: u16, value: &Value) -> Response {
    raw(
        status,
        Some("application/json"),
        serde_json::to_vec(value).expect("JSON values always serialize"),
    )
}

fn plain(text: String) -> Response {
    raw(200, Some("text/plain; charset=utf-8"), text.into_bytes())
}

/// An error of this front's own, in the shape FastAPI gave it.
fn detail(status: u16, message: &str) -> Response {
    json_reply(status, &json!({"detail": message}))
}

/// The answer to an upload without a `file`, as FastAPI wrote it.
fn missing_file() -> Response {
    json_reply(
        422,
        &json!({"detail": [{"type": "missing", "loc": ["body", "file"], "msg": "Field required", "input": null}]}),
    )
}

fn failed(failure: Failure) -> Response {
    match failure {
        Failure::Status { status, text } => detail(
            status,
            &format!("transcribe: {}", text.chars().take(300).collect::<String>()),
        ),
        Failure::Unreachable(why) => detail(502, &format!("transcribe: unreachable: {why}")),
        Failure::BadRequest(why) => detail(400, &why),
    }
}

fn alignment_failed(why: &str) -> Response {
    detail(502, &format!("ctc alignment: {why}"))
}

// ── Readiness and models ─────────────────────────────────────────────────────

async fn ready<T: Transcriber, S: Speak>(State(app): State<Arc<App<T, S>>>) -> Response {
    let (transcribe, tts) = tokio::join!(app.transcriber.healthy(), app.speak.ready());
    let mut capabilities = Vec::new();
    if transcribe {
        capabilities.extend(["asr", "diarization", "speech-translation"]);
    }
    if tts {
        capabilities.push("tts");
    }
    json_reply(
        200,
        &json!({
            "capabilities": capabilities, "device": "auto", "ready": transcribe && tts,
            "transcribe": transcribe.then(|| app.transcriber.model()),
            "tts": tts.then_some("magpietts"),
        }),
    )
}

async fn models<T: Transcriber, S: Speak>(State(app): State<Arc<App<T, S>>>) -> Response {
    // The ids `salm`, `diarization` and `speech-translation` are what callers send as
    // `model`; they are names on the wire, not ours to rename.
    let mut data = vec![
        json!({"capability": "transcription", "device": "auto", "id": "salm", "object": "model", "owned_by": "local",
               "speaker_diarization": true, "streaming": false, "word_timestamps": true, "root": app.transcriber.model()}),
        json!({"capability": "diarization", "device": "auto", "id": "diarization", "object": "model", "owned_by": "local"}),
        json!({"capability": "speech-translation", "device": "auto", "id": "speech-translation", "object": "model", "owned_by": "local"}),
    ];
    data.extend(
        app.speak
            .models()
            .await
            .into_iter()
            .filter(|m| m["id"] == "magpietts"),
    );
    json_reply(200, &json!({"object": "list", "data": data}))
}

// ── Dictation ────────────────────────────────────────────────────────────────

/// What Python's `bool()` says of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

async fn chat<T: Transcriber, S: Speak>(
    State(app): State<Arc<App<T, S>>>,
    body: Bytes,
) -> Response {
    let Ok(request) = serde_json::from_slice::<Value>(&body) else {
        return detail(400, "the request body must be JSON");
    };
    let stream = truthy(&request["stream"]);
    let reply = match app.transcriber.chat(request).await {
        Ok(reply) => reply,
        Err(failure) => return failed(failure),
    };
    if stream {
        // The whole event stream in one piece, untouched: dictation clips are short.
        return raw(
            reply.status,
            Some(reply.content_type.as_deref().unwrap_or("text/event-stream")),
            reply.body,
        );
    }
    if reply.status != 200 {
        return raw(reply.status, Some("application/json"), reply.body);
    }
    let mut answer = match serde_json::from_slice::<Value>(&reply.body) {
        Ok(answer @ Value::Object(_)) => answer,
        _ => return detail(502, "transcribe: the model's answer is not a JSON object"),
    };
    if let Some(choices) = answer.get_mut("choices").and_then(Value::as_array_mut) {
        for choice in choices {
            if let Some(content) = choice.pointer_mut("/message/content") {
                if let Some(cleaned) = content.as_str().filter(|s| !s.is_empty()).map(clean) {
                    *content = json!(cleaned);
                }
            }
        }
    }
    let transcription = match answer.pointer("/choices/0") {
        Some(first) => first
            .pointer("/message/content")
            .cloned()
            .unwrap_or(json!("")),
        None => json!(""),
    };
    answer["model"] = json!("salm");
    answer["transcription"] = transcription;
    json_reply(200, &answer)
}

// ── Uploads ──────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Upload {
    file: Option<Audio>,
    fields: HashMap<String, String>,
}

impl Upload {
    fn field<'a>(&'a self, name: &str, default: &'a str) -> &'a str {
        self.fields.get(name).map_or(default, String::as_str)
    }
}

async fn read_upload(form: Result<Multipart, MultipartRejection>) -> Result<Upload, Response> {
    // Not a form at all: to FastAPI that was simply a request without `file`.
    let Ok(mut form) = form else {
        return Err(missing_file());
    };
    let mut upload = Upload::default();
    loop {
        let field = match form.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => return Err(detail(e.status().as_u16(), &e.body_text())),
        };
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            let file_name = field
                .file_name()
                .filter(|n| !n.is_empty())
                .unwrap_or("a.wav")
                .to_string();
            let format = file_name.rsplit('.').next().unwrap_or("wav").to_string();
            match field.bytes().await {
                Ok(bytes) => {
                    upload.file = Some(Audio {
                        bytes: bytes.to_vec(),
                        format,
                    })
                }
                Err(e) => return Err(detail(e.status().as_u16(), &e.body_text())),
            }
        } else {
            match field.text().await {
                Ok(text) => drop(upload.fields.insert(name, text)),
                Err(e) => return Err(detail(e.status().as_u16(), &e.body_text())),
            }
        }
    }
    Ok(upload)
}

/// Who said what, in a way that survives the model under-tagging speakers: its word
/// alignment pushes untagged words of a second speaker into the first speaker's turns.
/// So: 1) speaker turns for the whole recording, 2) merge turns of one speaker less
/// than a second apart, 3) transcribe each turn on its own, as one speaker, with word
/// times, all at once, 4) shift the word times back to the recording's clock.
async fn diarized_words<T: Transcriber, S: Speak>(
    app: &App<T, S>,
    raw_audio: Vec<u8>,
    wording: &Task,
) -> Result<(Vec<Word>, f64), Response> {
    let wav16k = wav::to_wav16k(&app.ffmpeg, raw_audio)
        .await
        .map_err(|(status, why)| detail(status, &why))?;
    let duration = wav::seconds(&wav16k).unwrap_or(0.0);
    let whole = Audio {
        bytes: wav16k,
        format: "wav".into(),
    };
    let heard = app
        .transcriber
        .transcribe(&whole, &Task::Speakers, true)
        .await
        .map_err(failed)?;
    if let Some(why) = &heard.align_error {
        return Err(alignment_failed(why));
    }

    let mut turns = heard.turns;
    turns.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut merged: Vec<Turn> = Vec::new();
    for turn in turns {
        match merged.last_mut() {
            Some(last) if last.speaker == turn.speaker && turn.start - last.end < 1.0 => {
                last.end = last.end.max(turn.end)
            }
            _ => merged.push(turn),
        }
    }

    let whole = &whole;
    let per_turn = merged.iter().map(|turn| async move {
        let from = (turn.start - 0.15).max(0.0);
        let limit = if duration != 0.0 {
            duration
        } else {
            turn.end + 0.15
        };
        let to = limit.min(turn.end + 0.15);
        let piece = wav::slice(&whole.bytes, from, to)
            .ok_or_else(|| detail(500, "audio decode: ffmpeg did not return a WAV"))?;
        let piece = Audio {
            bytes: piece,
            format: "wav".into(),
        };
        let said = app
            .transcriber
            .transcribe(&piece, wording, true)
            .await
            .map_err(failed)?;
        let words = said.words.into_iter().map(|w| Word {
            word: w.word,
            start: round3(from + w.start),
            end: round3(from + w.end),
            speaker: turn.speaker,
        });
        Ok::<Vec<Word>, Response>(words.collect())
    });
    let words = try_join_all(per_turn)
        .await?
        .into_iter()
        .flatten()
        .collect();
    Ok((words, duration))
}

async fn transcriptions<T: Transcriber, S: Speak>(
    State(app): State<Arc<App<T, S>>>,
    form: Result<Multipart, MultipartRejection>,
) -> Response {
    let mut upload = match read_upload(form).await {
        Ok(upload) => upload,
        Err(response) => return response,
    };
    let Some(audio) = upload.file.take() else {
        return missing_file();
    };
    let format = upload.field("response_format", "json");
    let language = upload.field("language", "");
    let prompt = upload.field("prompt", "");
    let diarize = matches!(
        upload.field("diarization", "false").to_lowercase().as_str(),
        "1" | "true" | "yes"
    );
    // Verbatim unless the caller asks for `wording=clean`: the clean instruction also drops
    // digits said twice or more in a row (4 of 25 phone numbers and codes came back whole,
    // against 25 of 25 verbatim; 2026-11-10).
    let wording = if upload.field("wording", "").eq_ignore_ascii_case("clean") {
        Task::Clean
    } else {
        Task::Verbatim
    };

    if diarize {
        let (words, duration) = match diarized_words(&app, audio.bytes, &wording).await {
            Ok(found) => found,
            Err(response) => return response,
        };
        let text = clean(
            &words
                .iter()
                .map(|w| w.word.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        );
        return match format {
            "text" => plain(text),
            "verbose_json" => {
                let words: Vec<Value> = words
                    .iter()
                    .map(|w| json!({"confidence": 0, "start": w.start, "end": w.end, "speaker": w.speaker, "word": w.word}))
                    .collect();
                json_reply(
                    200,
                    &json!({"duration": duration, "language": language, "task": "transcribe", "text": text, "words": words}),
                )
            }
            _ => json_reply(200, &json!({"text": text})),
        };
    }

    let verbose = format == "verbose_json";
    let task = if prompt.is_empty() {
        wording
    } else {
        Task::Context(prompt.to_string())
    };
    let heard = match app.transcriber.transcribe(&audio, &task, verbose).await {
        Ok(heard) => heard,
        Err(failure) => return failed(failure),
    };
    let text = clean(&heard.text);
    if format == "text" {
        return plain(text);
    }
    if !verbose {
        return json_reply(200, &json!({"text": text}));
    }
    if let Some(why) = &heard.align_error {
        return alignment_failed(why);
    }
    // The speaker of a word comes from the audio-based turns, not the transcript's tags.
    // (Key order differs from the diarized answer above; both are as the Python front's.)
    let words: Vec<Value> = heard
        .words
        .iter()
        .map(|w| {
            let speaker = speaker_at(&heard.turns, w.start, w.end).unwrap_or(w.speaker);
            json!({"confidence": 0, "end": round3(w.end), "speaker": speaker, "start": round3(w.start), "word": w.word})
        })
        .collect();
    let duration = wav::seconds(&audio.bytes)
        .filter(|&seconds| seconds != 0.0)
        .or_else(|| heard.words.last().map(|w| round3(w.end)))
        .unwrap_or(0.0);
    json_reply(
        200,
        &json!({"duration": duration, "language": language, "task": "transcribe", "text": text, "words": words}),
    )
}

async fn diarizations<T: Transcriber, S: Speak>(
    State(app): State<Arc<App<T, S>>>,
    form: Result<Multipart, MultipartRejection>,
) -> Response {
    let audio = match read_upload(form).await {
        Ok(Upload {
            file: Some(audio), ..
        }) => audio,
        Ok(_) => return missing_file(),
        Err(response) => return response,
    };
    let heard = match app
        .transcriber
        .transcribe(&audio, &Task::Speakers, true)
        .await
    {
        Ok(heard) => heard,
        Err(failure) => return failed(failure),
    };
    if let Some(why) = &heard.align_error {
        return alignment_failed(why);
    }
    let segments: Vec<Value> = heard
        .turns
        .iter()
        .map(|t| json!({"end": t.end, "speaker": t.speaker, "start": t.start}))
        .collect();
    json_reply(200, &json!({"segments": segments}))
}

async fn translations<T: Transcriber, S: Speak>(
    State(app): State<Arc<App<T, S>>>,
    form: Result<Multipart, MultipartRejection>,
) -> Response {
    let mut upload = match read_upload(form).await {
        Ok(upload) => upload,
        Err(response) => return response,
    };
    let Some(audio) = upload.file.take() else {
        return missing_file();
    };
    let language = match upload.field("language", "English") {
        "" => "English",
        language => language,
    };
    let heard = match app
        .transcriber
        .transcribe(&audio, &Task::Translate(language.to_string()), false)
        .await
    {
        Ok(heard) => heard,
        Err(failure) => return failed(failure),
    };
    let text = clean(&heard.text);
    if upload.field("response_format", "json") == "text" {
        plain(text)
    } else {
        json_reply(200, &json!({"text": text}))
    }
}

// ── Handed on as they are ────────────────────────────────────────────────────

fn content_type_of(headers: &HeaderMap) -> &str {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
}

/// Text to speech: the request goes to the text-to-speech server untouched.
async fn speech<T: Transcriber, S: Speak>(
    State(app): State<Arc<App<T, S>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match app
        .speak
        .speech(content_type_of(&headers), body.to_vec())
        .await
    {
        Ok(reply) => raw(
            reply.status,
            Some(reply.content_type.as_deref().unwrap_or("audio/wav")),
            reply.body,
        ),
        Err(Failure::Unreachable(why)) => detail(502, &format!("speech: unreachable: {why}")),
        Err(failure) => failed(failure),
    }
}

/// Every other `GET` or `POST` goes to the model server at the same path. The query
/// string is not passed on (the Python front dropped it too).
async fn forward<T: Transcriber, S: Speak>(
    State(app): State<Arc<App<T, S>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if method != Method::GET && method != Method::POST {
        return detail(405, "Method Not Allowed");
    }
    match app
        .transcriber
        .forward(
            method.as_str(),
            uri.path(),
            content_type_of(&headers),
            body.to_vec(),
        )
        .await
    {
        Ok(reply) => raw(reply.status, reply.content_type.as_deref(), reply.body),
        Err(failure) => failed(failure),
    }
}
