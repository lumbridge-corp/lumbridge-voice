//! The HTTP API against the mock model, and the adapters against stand-in servers.
//!
//! `tests/fixtures/observed/` holds real answers of the earlier Python front
//! (2026-11-09). Where a test says "observed", this front must answer byte for byte
//! the same when the mock model says what the real model said.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use http_body_util::BodyExt;
use lumbridge_voice::adapter::{
    Audio, Call, Failure, MockSpeak, MockTranscriber, Reply, Speak, Task, Transcriber, Transcript,
    Turn, Word,
};
use lumbridge_voice::config::Config;
use lumbridge_voice::server::{router, App};
use lumbridge_voice::speak::HttpSpeak;
use lumbridge_voice::vllm::{parse_transcript, Vllm};
use lumbridge_voice::wav;
use serde_json::{json, Value};
use tower::ServiceExt;

const MODEL: &str = "nvidia/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B-NVFP4";
const OBSERVED_READY: &str = include_str!("fixtures/observed/ready.json");
const OBSERVED_MODELS: &str = include_str!("fixtures/observed/models.json");
const OBSERVED_VERBOSE: &str = include_str!("fixtures/observed/transcription-verbose.json");
const OBSERVED_DIARIZED: &str =
    include_str!("fixtures/observed/transcription-diarized-verbose.json");
const OBSERVED_DIARIZATIONS: &str = include_str!("fixtures/observed/diarizations.json");
const OBSERVED_CHAT: &str = include_str!("fixtures/observed/chat.json");
/// The real model's answer with word times and speaker turns.
const MODEL_ANSWER: &str = include_str!("fixtures/observed/chat-with-word-times.json");
/// FastAPI's answer to an upload without `file` (observed).
const MISSING_FILE: &str =
    r#"{"detail":[{"type":"missing","loc":["body","file"],"msg":"Field required","input":null}]}"#;

const TRANSCRIPTIONS: &str = "/v1/audio/transcriptions";
const DIARIZATIONS: &str = "/v1/audio/diarizations";
const TRANSLATIONS: &str = "/v1/audio/translations";
const CHAT: &str = "/v1/chat/completions";

// ── Helpers ──────────────────────────────────────────────────────────────────

fn app<T: Transcriber, S: Speak>(transcriber: T, speak: S) -> Arc<App<T, S>> {
    Arc::new(App {
        transcriber,
        speak,
        ffmpeg: "ffmpeg".into(),
        max_upload_bytes: 64 * 1024 * 1024,
    })
}

/// A mock model that says what the real model said about the test clip.
fn model_that_heard_the_clip() -> MockTranscriber {
    let said = parse_transcript(&serde_json::from_str(MODEL_ANSWER).unwrap());
    MockTranscriber {
        model: MODEL.into(),
        transcript: Transcript {
            text: "<spk:0> Testing one to three.".into(),
            ..said
        },
        ..Default::default()
    }
}

/// Magpie's entry in its own model list (observed through the Python front).
fn magpie_models() -> Vec<Value> {
    let all: Value = serde_json::from_str(OBSERVED_MODELS).unwrap();
    all["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["id"] == "magpietts")
        .cloned()
        .collect()
}

fn mock_speak() -> MockSpeak {
    MockSpeak {
        models: magpie_models(),
        ..Default::default()
    }
}

fn silence(rate: u32, frames: usize) -> Vec<u8> {
    wav::write(&vec![0u8; frames * 2], rate)
}

struct Answer {
    status: StatusCode,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Answer {
    fn text(&self) -> &str {
        std::str::from_utf8(&self.body).expect("text body")
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("JSON body")
    }

    fn is_json(&self) -> bool {
        self.content_type.as_deref() == Some("application/json")
    }
}

async fn send<T: Transcriber, S: Speak>(
    app: &Arc<App<T, S>>,
    method: &str,
    path: &str,
    content_type: &str,
    body: Vec<u8>,
) -> Answer {
    let mut request = Request::builder().method(method).uri(path);
    if !content_type.is_empty() {
        request = request.header(header::CONTENT_TYPE, content_type);
    }
    let response = router(app.clone())
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    Answer {
        status: response.status(),
        content_type: response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string()),
        body: response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    }
}

async fn get_path<T: Transcriber, S: Speak>(app: &Arc<App<T, S>>, path: &str) -> Answer {
    send(app, "GET", path, "", Vec::new()).await
}

/// A `multipart/form-data` upload: `file` (named `file_name`) plus text fields.
async fn upload<T: Transcriber, S: Speak>(
    app: &Arc<App<T, S>>,
    path: &str,
    file: Option<(&str, &[u8])>,
    fields: &[(&str, &str)],
) -> Answer {
    let boundary = "lumbridge-test-boundary";
    let mut body = Vec::new();
    if let Some((file_name, bytes)) = file {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    send(
        app,
        "POST",
        path,
        &format!("multipart/form-data; boundary={boundary}"),
        body,
    )
    .await
}

async fn post_json<T: Transcriber, S: Speak>(
    app: &Arc<App<T, S>>,
    path: &str,
    body: &str,
) -> Answer {
    send(
        app,
        "POST",
        path,
        "application/json",
        body.as_bytes().to_vec(),
    )
    .await
}

// ── GET /ready, /health ──────────────────────────────────────────────────────

#[tokio::test]
async fn ready_and_health_answer_as_observed() {
    let app = app(model_that_heard_the_clip(), mock_speak());
    for path in ["/ready", "/health"] {
        let a = get_path(&app, path).await;
        assert_eq!(a.status, StatusCode::OK);
        assert!(a.is_json());
        assert_eq!(a.text(), OBSERVED_READY, "{path}");
    }
    // What the model manager's health check looks for.
    assert!(OBSERVED_READY.contains("\"ready\":true"));
}

#[tokio::test]
async fn ready_says_which_half_is_down() {
    let no_model = app(
        MockTranscriber {
            healthy: false,
            ..model_that_heard_the_clip()
        },
        mock_speak(),
    );
    let a = get_path(&no_model, "/ready").await;
    assert_eq!(a.status, StatusCode::OK, "always 200; callers read `ready`");
    assert_eq!(
        a.text(),
        r#"{"capabilities":["tts"],"device":"auto","ready":false,"transcribe":null,"tts":"magpietts"}"#
    );

    let no_tts = app(
        model_that_heard_the_clip(),
        MockSpeak {
            ready: false,
            ..mock_speak()
        },
    );
    let a = get_path(&no_tts, "/ready").await;
    assert_eq!(
        a.text(),
        format!(
            r#"{{"capabilities":["asr","diarization","speech-translation"],"device":"auto","ready":false,"transcribe":"{MODEL}","tts":null}}"#
        )
    );
}

// ── GET /v1/models ───────────────────────────────────────────────────────────

#[tokio::test]
async fn models_answers_as_observed() {
    let a = get_path(
        &app(model_that_heard_the_clip(), mock_speak()),
        "/v1/models",
    )
    .await;
    assert_eq!(a.status, StatusCode::OK);
    assert!(a.is_json());
    assert_eq!(a.text(), OBSERVED_MODELS);

    // Without text to speech, only the three fixed entries.
    let a = get_path(
        &app(model_that_heard_the_clip(), MockSpeak::default()),
        "/v1/models",
    )
    .await;
    let ids: Vec<String> = a.json()["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, ["salm", "diarization", "speech-translation"]);
}

// ── POST /v1/chat/completions ────────────────────────────────────────────────

/// What vLLM would have answered for the observed dictation: the observed body without
/// the front's changes (`model`, `transcription`), and with tags left in the text.
fn model_chat_answer() -> Vec<u8> {
    let mut answer: Value = serde_json::from_str(OBSERVED_CHAT).unwrap();
    answer
        .as_object_mut()
        .unwrap()
        .shift_remove("transcription");
    answer["model"] = json!(MODEL);
    answer["choices"][0]["message"]["content"] =
        json!("<spk:0> Testing  one two three.<|endoftext|>");
    serde_json::to_vec(&answer).unwrap()
}

#[tokio::test]
async fn chat_answers_as_observed() {
    let model = MockTranscriber {
        chat_reply: Reply {
            status: 200,
            content_type: Some("application/json".into()),
            body: model_chat_answer(),
        },
        ..model_that_heard_the_clip()
    };
    let app = app(model, mock_speak());
    let asked = json!({"model": "salm", "messages": [{"role": "user", "content": [
        {"type": "text", "text": "Transcribe with punctuation and capitalization."},
        {"type": "input_audio", "input_audio": {"data": "UklGRg==", "format": "wav"}}]}]});
    let a = post_json(&app, CHAT, &asked.to_string()).await;
    assert_eq!(a.status, StatusCode::OK);
    assert!(a.is_json());
    assert_eq!(a.text(), OBSERVED_CHAT);
    // The parts callers read.
    assert_eq!(
        a.json()["choices"][0]["message"]["content"],
        "Testing one two three."
    );
    assert_eq!(a.json()["transcription"], "Testing one two three.");
    assert_eq!(a.json()["model"], "salm");
    assert_eq!(
        *app.transcriber.chats.lock().unwrap(),
        [asked],
        "the adapter gets the caller's request"
    );
}

#[tokio::test]
async fn chat_passes_streams_and_errors_through_untouched() {
    let events =
        b"data: {\"choices\":[{\"delta\":{\"content\":\"<spk:0> Hi\"}}]}\n\ndata: [DONE]\n\n"
            .to_vec();
    let stream = MockTranscriber {
        chat_reply: Reply {
            status: 200,
            content_type: Some("text/event-stream; charset=utf-8".into()),
            body: events.clone(),
        },
        ..Default::default()
    };
    let a = post_json(
        &app(stream, mock_speak()),
        CHAT,
        r#"{"stream":true,"messages":[]}"#,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(
        a.content_type.as_deref(),
        Some("text/event-stream; charset=utf-8")
    );
    assert_eq!(a.body, events);

    // vLLM's real answer to a request without messages (observed, status 400).
    let refusal = r#"{"error":{"message":"Cannot apply chat template to an empty conversation. Provide at least one message.","type":"BadRequestError","param":null,"code":400}}"#;
    let refusing = MockTranscriber {
        chat_reply: Reply {
            status: 400,
            content_type: None,
            body: refusal.into(),
        },
        ..Default::default()
    };
    let a = post_json(&app(refusing, mock_speak()), CHAT, r#"{"messages":[]}"#).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST);
    assert!(a.is_json());
    assert_eq!(a.text(), refusal);
}

#[tokio::test]
async fn chat_without_choices_has_an_empty_transcription() {
    let empty = MockTranscriber {
        chat_reply: Reply {
            status: 200,
            content_type: None,
            body: br#"{"model":"x","choices":[]}"#.to_vec(),
        },
        ..Default::default()
    };
    let a = post_json(&app(empty, mock_speak()), CHAT, "{}").await;
    assert_eq!(
        a.text(),
        r#"{"model":"salm","choices":[],"transcription":""}"#
    );

    // Not JSON: this front says so (the Python front answered 500 with no reason).
    let a = post_json(&app(MockTranscriber::default(), mock_speak()), CHAT, "nope").await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST);
    assert_eq!(a.text(), r#"{"detail":"the request body must be JSON"}"#);
}

// ── POST /v1/audio/transcriptions ────────────────────────────────────────────

#[tokio::test]
async fn transcription_json_and_text_answer_as_observed() {
    let app = app(model_that_heard_the_clip(), mock_speak());
    let clip = silence(22_050, 34_816);

    let a = upload(&app, TRANSCRIPTIONS, Some(("tiny.wav", &clip)), &[]).await;
    assert_eq!(a.status, StatusCode::OK);
    assert!(a.is_json());
    assert_eq!(a.text(), r#"{"text":"Testing one to three."}"#); // observed

    let a = upload(
        &app,
        TRANSCRIPTIONS,
        Some(("tiny.wav", &clip)),
        &[("response_format", "text")],
    )
    .await;
    assert_eq!(a.content_type.as_deref(), Some("text/plain; charset=utf-8")); // observed
    assert_eq!(a.text(), "Testing one to three."); // observed

    // An unknown format behaves as json; `model` is ignored.
    let a = upload(
        &app,
        TRANSCRIPTIONS,
        Some(("tiny.wav", &clip)),
        &[("response_format", "srt"), ("model", "whisper-1")],
    )
    .await;
    assert_eq!(a.text(), r#"{"text":"Testing one to three."}"#);

    let calls = app.transcriber.calls.lock().unwrap().clone();
    let plain = Call {
        audio: Audio {
            bytes: clip,
            format: "wav".into(),
        },
        task: Task::Verbatim,
        timestamps: false,
    };
    assert_eq!(
        calls,
        [plain.clone(), plain.clone(), plain],
        "one pass each, no word times asked for"
    );
}

#[tokio::test]
async fn transcription_verbose_answers_as_observed() {
    let app = app(model_that_heard_the_clip(), mock_speak());
    // Same length as the observed clip: 34,816 samples at 22050 Hz.
    let clip = silence(22_050, 34_816);
    let a = upload(
        &app,
        TRANSCRIPTIONS,
        Some(("tiny.wav", &clip)),
        &[("response_format", "verbose_json"), ("language", "en")],
    )
    .await;
    assert_eq!(a.status, StatusCode::OK);
    assert!(a.is_json());
    assert_eq!(a.text(), OBSERVED_VERBOSE);
    let calls = app.transcriber.calls.lock().unwrap();
    assert_eq!((calls.len(), calls[0].timestamps), (1, true));
}

#[tokio::test]
async fn wording_is_verbatim_unless_clean_is_asked_for() {
    let app = app(model_that_heard_the_clip(), mock_speak());
    upload(&app, TRANSCRIPTIONS, Some(("a.wav", b"clip")), &[]).await;
    upload(
        &app,
        TRANSCRIPTIONS,
        Some(("a.wav", b"clip")),
        &[("wording", "Clean")],
    )
    .await;
    upload(
        &app,
        TRANSCRIPTIONS,
        Some(("a.wav", b"clip")),
        &[("wording", "anything else")],
    )
    .await;
    // Terms given with `prompt` win over the wording, as before.
    upload(
        &app,
        TRANSCRIPTIONS,
        Some(("a.wav", b"clip")),
        &[("wording", "clean"), ("prompt", "Ada")],
    )
    .await;
    let tasks: Vec<Task> = app
        .transcriber
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.task.clone())
        .collect();
    assert_eq!(
        tasks,
        [
            Task::Verbatim,
            Task::Clean,
            Task::Verbatim,
            Task::Context("Ada".into())
        ]
    );
}

#[tokio::test]
async fn transcription_prompt_becomes_context_terms() {
    let app = app(model_that_heard_the_clip(), mock_speak());
    upload(
        &app,
        TRANSCRIPTIONS,
        Some(("a.mp3", b"not really mp3")),
        &[("prompt", "Ada, Lumbridge")],
    )
    .await;
    let calls = app.transcriber.calls.lock().unwrap();
    assert_eq!(calls[0].task, Task::Context("Ada, Lumbridge".into()));
    assert_eq!(
        calls[0].audio.format, "mp3",
        "the file name's extension names the format"
    );
}

#[tokio::test]
async fn transcription_duration_falls_back_to_the_last_word() {
    let app = app(model_that_heard_the_clip(), mock_speak());
    let a = upload(
        &app,
        TRANSCRIPTIONS,
        Some(("a.mp3", b"not a wav")),
        &[("response_format", "verbose_json")],
    )
    .await;
    assert_eq!(a.json()["duration"], json!(1.51));
    assert_eq!(a.json()["language"], json!(""));

    let silent = MockTranscriber {
        transcript: Transcript::default(),
        ..Default::default()
    };
    let a = upload(
        &crate::app(silent, mock_speak()),
        TRANSCRIPTIONS,
        Some(("a.mp3", b"x")),
        &[("response_format", "verbose_json")],
    )
    .await;
    assert_eq!(
        a.text(),
        r#"{"duration":0.0,"language":"","task":"transcribe","text":"","words":[]}"#
    );
}

#[tokio::test]
async fn word_speakers_come_from_the_turns_not_the_tags() {
    let word = |word: &str, start: f64, end: f64| Word {
        word: word.into(),
        start,
        end,
        speaker: 1,
    };
    let model = MockTranscriber {
        transcript: Transcript {
            text: "<spk:0> Hello there. Hi!".into(),
            // The model tagged everything speaker 1, but heard two voices.
            words: vec![
                word("Hello", 0.1, 0.4),
                word("there.", 0.45, 0.9),
                word("Hi!", 1.3, 1.6),
            ],
            turns: vec![
                Turn {
                    start: 0.0,
                    end: 1.0,
                    speaker: 1,
                },
                Turn {
                    start: 1.2,
                    end: 1.7,
                    speaker: 2,
                },
            ],
            align_error: None,
        },
        ..Default::default()
    };
    let a = upload(
        &app(model, mock_speak()),
        TRANSCRIPTIONS,
        Some(("a.wav", &silence(16_000, 32_000))),
        &[("response_format", "verbose_json")],
    )
    .await;
    let speakers: Vec<u64> = a.json()["words"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["speaker"].as_u64().unwrap())
        .collect();
    assert_eq!(speakers, [1, 1, 2]);
    assert_eq!(a.json()["duration"], json!(2.0));
}

#[tokio::test]
async fn diarized_transcription_answers_as_observed() {
    let app = app(model_that_heard_the_clip(), mock_speak());
    // Already 16 kHz mono, so no ffmpeg is needed. 1.6 seconds.
    let clip = silence(16_000, 25_600);
    let a = upload(
        &app,
        TRANSCRIPTIONS,
        Some(("tiny.wav", &clip)),
        &[("response_format", "verbose_json"), ("diarization", "true")],
    )
    .await;
    assert_eq!(a.status, StatusCode::OK);
    assert!(a.is_json());
    // The observed answer, except for its duration: the Python front reported
    // 134217.7279375 s for the 1.58 s clip (docs/API.md, "Known fault"). This front
    // reports the real length.
    assert!(OBSERVED_DIARIZED.contains(r#""duration":134217.7279375,"#));
    assert_eq!(
        a.text(),
        OBSERVED_DIARIZED.replace(r#""duration":134217.7279375,"#, r#""duration":1.6,"#)
    );

    // First the whole recording for the turns, then the one turn on its own.
    let calls = app.transcriber.calls.lock().unwrap().clone();
    let whole = Audio {
        bytes: clip,
        format: "wav".into(),
    };
    assert_eq!(
        calls,
        [
            Call {
                audio: whole.clone(),
                task: Task::Speakers,
                timestamps: true
            },
            // The turn is 0.0 to 1.5 s; with 0.15 s of padding, capped at the end: all of it.
            Call {
                audio: whole,
                task: Task::Verbatim,
                timestamps: true
            },
        ]
    );

    for (format, want) in [
        ("json", r#"{"text":"Testing one to three."}"#),
        ("text", "Testing one to three."),
    ] {
        let a = upload(
            &app,
            TRANSCRIPTIONS,
            Some(("tiny.wav", &silence(16_000, 25_600))),
            &[("response_format", format), ("diarization", "YES")],
        )
        .await;
        assert_eq!(a.text(), want, "{format}");
    }
}

#[tokio::test]
async fn diarized_transcription_works_turn_by_turn() {
    let model = MockTranscriber {
        transcript: Transcript {
            text: "hi".into(),
            // Every piece of audio "contains" this one word, 0.2 s in.
            words: vec![Word {
                word: "hi".into(),
                start: 0.2,
                end: 0.4,
                speaker: 1,
            }],
            // Out of order, and two turns of speaker 1 that belong together.
            turns: vec![
                Turn {
                    start: 3.5,
                    end: 4.0,
                    speaker: 2,
                },
                Turn {
                    start: 0.0,
                    end: 1.0,
                    speaker: 1,
                },
                Turn {
                    start: 1.2,
                    end: 2.0,
                    speaker: 1,
                },
                Turn {
                    start: 4.2,
                    end: 5.0,
                    speaker: 1,
                },
            ],
            align_error: None,
        },
        ..Default::default()
    };
    let app = app(model, mock_speak());
    let clip = silence(16_000, 80_000); // 5 seconds
    let a = upload(
        &app,
        TRANSCRIPTIONS,
        Some(("meeting.wav", &clip)),
        &[("response_format", "verbose_json"), ("diarization", "1")],
    )
    .await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(
        a.json(),
        json!({"duration": 5.0, "language": "", "task": "transcribe", "text": "hi hi hi", "words": [
            {"confidence": 0, "start": 0.2, "end": 0.4, "speaker": 1, "word": "hi"},
            {"confidence": 0, "start": 3.55, "end": 3.75, "speaker": 2, "word": "hi"},
            {"confidence": 0, "start": 4.25, "end": 4.45, "speaker": 1, "word": "hi"},
        ]})
    );
    // Key order inside a word, as the Python front wrote it on this path.
    assert!(a
        .text()
        .contains(r#"{"confidence":0,"start":3.55,"end":3.75,"speaker":2,"word":"hi"}"#));

    let calls = app.transcriber.calls.lock().unwrap();
    assert_eq!(
        calls.len(),
        4,
        "the whole recording, then three merged turns"
    );
    assert_eq!(calls[0].task, Task::Speakers);
    // Turns 0–2.0, 3.5–4.0 and 4.2–5.0 with 0.15 s around each, capped at 0 and 5 s.
    for (call, seconds) in calls[1..].iter().zip([2.15, 0.8, 0.95]) {
        assert_eq!((&call.task, call.timestamps), (&Task::Verbatim, true));
        let got = wav::seconds(&call.audio.bytes).unwrap();
        assert!(
            (got - seconds).abs() < 0.001,
            "piece of {got} s, expected {seconds}"
        );
    }
}

#[tokio::test]
async fn nothing_heard_is_an_empty_answer() {
    let silent = MockTranscriber {
        transcript: Transcript::default(),
        ..Default::default()
    };
    let app = app(silent, mock_speak());
    let a = upload(
        &app,
        TRANSCRIPTIONS,
        Some(("a.wav", &silence(16_000, 16_000))),
        &[("response_format", "verbose_json"), ("diarization", "true")],
    )
    .await;
    assert_eq!(
        a.text(),
        r#"{"duration":1.0,"language":"","task":"transcribe","text":"","words":[]}"#
    );
    assert_eq!(
        app.transcriber.calls.lock().unwrap().len(),
        1,
        "no turns, no second pass"
    );
}

// ── POST /v1/audio/diarizations ──────────────────────────────────────────────

#[tokio::test]
async fn diarizations_answers_as_observed() {
    let app = app(model_that_heard_the_clip(), mock_speak());
    let clip = silence(22_050, 34_816);
    let a = upload(&app, DIARIZATIONS, Some(("tiny.wav", &clip)), &[]).await;
    assert_eq!(a.status, StatusCode::OK);
    assert!(a.is_json());
    assert_eq!(a.text(), OBSERVED_DIARIZATIONS);
    let calls = app.transcriber.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        [Call {
            audio: Audio {
                bytes: clip,
                format: "wav".into()
            },
            task: Task::Speakers,
            timestamps: true
        }]
    );
}

// ── POST /v1/audio/translations ──────────────────────────────────────────────

#[tokio::test]
async fn translations_answers_as_observed() {
    let app = app(model_that_heard_the_clip(), mock_speak());
    let clip = silence(22_050, 34_816);
    let a = upload(&app, TRANSLATIONS, Some(("tiny.wav", &clip)), &[]).await;
    assert_eq!(a.status, StatusCode::OK);
    assert!(a.is_json());
    assert_eq!(a.text(), r#"{"text":"Testing one to three."}"#); // observed

    let a = upload(
        &app,
        TRANSLATIONS,
        Some(("tiny.wav", &clip)),
        &[("language", "German"), ("response_format", "text")],
    )
    .await;
    assert_eq!(a.content_type.as_deref(), Some("text/plain; charset=utf-8"));
    assert_eq!(a.text(), "Testing one to three.");
    upload(
        &app,
        TRANSLATIONS,
        Some(("tiny.wav", &clip)),
        &[("language", "")],
    )
    .await;

    let asked: Vec<(Task, bool)> = app
        .transcriber
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| (c.task.clone(), c.timestamps))
        .collect();
    assert_eq!(
        asked,
        [
            (Task::Translate("English".into()), false),
            (Task::Translate("German".into()), false),
            (Task::Translate("English".into()), false)
        ]
    );
}

// ── Uploads that cannot be answered ──────────────────────────────────────────

#[tokio::test]
async fn an_upload_without_a_file_is_a_422_as_observed() {
    let app = app(model_that_heard_the_clip(), mock_speak());
    for path in [TRANSCRIPTIONS, DIARIZATIONS, TRANSLATIONS] {
        // Observed on /v1/audio/transcriptions (a form with only `model`) and on
        // /v1/audio/diarizations (no body at all).
        let form_without_file = upload(&app, path, None, &[("model", "salm")]).await;
        let no_body = send(&app, "POST", path, "", Vec::new()).await;
        let json_body = post_json(&app, path, r#"{"file":"x"}"#).await;
        for a in [form_without_file, no_body, json_body] {
            assert_eq!(a.status, StatusCode::UNPROCESSABLE_ENTITY, "{path}");
            assert!(a.is_json());
            assert_eq!(a.text(), MISSING_FILE, "{path}");
        }
    }
    assert!(app.transcriber.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn model_failures_keep_the_python_fronts_shape() {
    let clip = silence(16_000, 16_000);
    let failing = |failure: Failure| {
        app(
            MockTranscriber {
                fail: Some(failure),
                ..Default::default()
            },
            mock_speak(),
        )
    };

    // The model answered with an error: its status, and the first 300 characters.
    let busy = failing(Failure::Status {
        status: 503,
        text: "x".repeat(400),
    });
    for (path, fields) in [
        (TRANSCRIPTIONS, vec![]),
        (TRANSCRIPTIONS, vec![("diarization", "true")]),
        (DIARIZATIONS, vec![]),
        (TRANSLATIONS, vec![]),
    ] {
        let a = upload(&busy, path, Some(("a.wav", &clip)), &fields).await;
        assert_eq!(a.status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert_eq!(
            a.text(),
            format!(r#"{{"detail":"transcribe: {}"}}"#, "x".repeat(300)),
            "{path}"
        );
    }

    // The model could not be reached. (The Python front: 500, no reason given.)
    let down = failing(Failure::Unreachable("connection refused".into()));
    let a = upload(&down, TRANSCRIPTIONS, Some(("a.wav", &clip)), &[]).await;
    assert_eq!(a.status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        a.text(),
        r#"{"detail":"transcribe: unreachable: connection refused"}"#
    );
}

#[tokio::test]
async fn failed_word_alignment_is_a_502_only_where_times_were_asked_for() {
    let model = || MockTranscriber {
        transcript: Transcript {
            text: "Hello.".into(),
            align_error: Some("audio too short".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let clip = silence(16_000, 16_000);
    let want = r#"{"detail":"ctc alignment: audio too short"}"#;
    for (path, fields) in [
        (TRANSCRIPTIONS, vec![("response_format", "verbose_json")]),
        (TRANSCRIPTIONS, vec![("diarization", "true")]),
        (DIARIZATIONS, vec![]),
    ] {
        let a = upload(
            &app(model(), mock_speak()),
            path,
            Some(("a.wav", &clip)),
            &fields,
        )
        .await;
        assert_eq!(a.status, StatusCode::BAD_GATEWAY, "{path} {fields:?}");
        assert_eq!(a.text(), want, "{path} {fields:?}");
    }
    let a = upload(
        &app(model(), mock_speak()),
        TRANSCRIPTIONS,
        Some(("a.wav", &clip)),
        &[],
    )
    .await;
    assert_eq!(a.text(), r#"{"text":"Hello."}"#);
}

#[tokio::test]
async fn an_upload_over_the_limit_is_refused() {
    let small = Arc::new(App {
        transcriber: MockTranscriber::default(),
        speak: mock_speak(),
        ffmpeg: "ffmpeg".into(),
        max_upload_bytes: 1000,
    });
    let a = upload(
        &small,
        TRANSCRIPTIONS,
        Some(("a.wav", &silence(16_000, 16_000))),
        &[],
    )
    .await;
    assert_eq!(a.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(small.transcriber.calls.lock().unwrap().is_empty());
}

// ── POST /v1/audio/speech ────────────────────────────────────────────────────

#[tokio::test]
async fn speech_is_handed_to_the_text_to_speech_server_untouched() {
    let app = app(MockTranscriber::default(), mock_speak());
    // What the Mac app sends.
    let asked = r#"{"input":"Testing one two three.","voice":"Leo","language":"en-US","response_format":"wav"}"#;
    let a = post_json(&app, "/v1/audio/speech", asked).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.content_type.as_deref(), Some("audio/wav")); // observed
    assert_eq!(&a.body[..4], b"RIFF");
    assert_eq!(
        *app.speak.calls.lock().unwrap(),
        [("application/json".to_string(), asked.as_bytes().to_vec())]
    );

    // Magpie's real refusal of an unknown voice (observed, status 400) comes back as it is.
    let refusal = r#"{"error":{"code":null,"message":"unknown voice_name 'Nobody'","param":null,"type":"invalid_request_error"}}"#;
    let refusing = MockSpeak {
        reply: Ok(Reply {
            status: 400,
            content_type: Some("application/json".into()),
            body: refusal.into(),
        }),
        ..Default::default()
    };
    let a = post_json(
        &crate::app(MockTranscriber::default(), refusing),
        "/v1/audio/speech",
        r#"{"input":"hi","voice":"Nobody"}"#,
    )
    .await;
    assert_eq!((a.status, a.text()), (StatusCode::BAD_REQUEST, refusal));
    assert!(a.is_json());

    // No content type from upstream: audio/wav, as in the Python front.
    let bare = MockSpeak {
        reply: Ok(Reply {
            status: 200,
            content_type: None,
            body: vec![1, 2],
        }),
        ..Default::default()
    };
    let a = post_json(
        &crate::app(MockTranscriber::default(), bare),
        "/v1/audio/speech",
        "{}",
    )
    .await;
    assert_eq!(a.content_type.as_deref(), Some("audio/wav"));

    let down = MockSpeak {
        reply: Err(Failure::Unreachable("connection refused".into())),
        ..Default::default()
    };
    let a = post_json(
        &crate::app(MockTranscriber::default(), down),
        "/v1/audio/speech",
        "{}",
    )
    .await;
    assert_eq!(a.status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        a.text(),
        r#"{"detail":"speech: unreachable: connection refused"}"#
    );
}

// ── Any other path ───────────────────────────────────────────────────────────

#[tokio::test]
async fn other_paths_go_to_the_model_server() {
    let model = MockTranscriber {
        // vLLM's real answer to GET /version (observed through the Python front).
        forward_reply: Reply {
            status: 200,
            content_type: Some("application/json".into()),
            body: br#"{"version":"0.32.0"}"#.to_vec(),
        },
        ..Default::default()
    };
    let app = app(model, mock_speak());
    let a = get_path(&app, "/version?verbose=1").await;
    assert_eq!(
        (a.status, a.text()),
        (StatusCode::OK, r#"{"version":"0.32.0"}"#)
    );
    assert!(a.is_json());
    post_json(&app, "/tokenize", r#"{"prompt":"hi"}"#).await;
    // A known path with the other method is forwarded too, as in the Python front.
    get_path(&app, TRANSCRIPTIONS).await;
    send(&app, "POST", "/ready", "text/plain", b"x".to_vec()).await;

    let forwards = app.transcriber.forwards.lock().unwrap().clone();
    let f = |method: &str, path: &str, content_type: &str, body: &str| {
        (
            method.to_string(),
            path.to_string(),
            content_type.to_string(),
            body.as_bytes().to_vec(),
        )
    };
    assert_eq!(
        forwards,
        [
            f("GET", "/version", "application/json", ""), // no query string, as in the Python front
            f(
                "POST",
                "/tokenize",
                "application/json",
                r#"{"prompt":"hi"}"#
            ),
            f("GET", TRANSCRIPTIONS, "application/json", ""),
            f("POST", "/ready", "text/plain", "x"),
        ]
    );

    // Other methods: observed for PUT /version.
    for method in ["PUT", "DELETE", "PATCH"] {
        let a = send(&app, method, "/version", "", Vec::new()).await;
        assert_eq!(a.status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(a.text(), r#"{"detail":"Method Not Allowed"}"#);
    }
    assert_eq!(app.transcriber.forwards.lock().unwrap().len(), 4);
}

// ── The vLLM adapter against a stand-in vLLM ─────────────────────────────────

type Seen = Arc<Mutex<Vec<Value>>>;

/// A stand-in vLLM: `/health`, `/version`, and `/v1/chat/completions` answering with
/// `reply` and remembering each request.
async fn fake_vllm(reply: (StatusCode, String)) -> (String, Seen) {
    let seen = Seen::default();
    let log = seen.clone();
    let service = Router::new()
        .route("/health", get(|| async { "" }))
        .route(
            "/version",
            get(|| async { Json(json!({"version": "0.32.0"})) }),
        )
        .route(
            CHAT,
            post(move |Json(asked): Json<Value>| {
                log.lock().unwrap().push(asked);
                let (status, body) = reply.clone();
                async move { (status, [(header::CONTENT_TYPE, "application/json")], body) }
            }),
        );
    (serve(service).await, seen)
}

/// A stand-in Magpie that answers what the real one answered (observed).
async fn fake_magpie() -> String {
    let models = json!({"data": magpie_models(), "object": "list"});
    let service =
        Router::new()
            .route(
                "/ready",
                get(|| async {
                    Json(json!({"capabilities": ["tts"], "device": "auto", "ready": true}))
                }),
            )
            .route(
                "/v1/models",
                get(move || {
                    let models = models.clone();
                    async move { Json(models) }
                }),
            )
            .route(
                "/v1/audio/speech",
                post(|body: axum::body::Bytes| async move {
                    ([(header::CONTENT_TYPE, "audio/pcm")], body)
                }),
            );
    serve(service).await
}

async fn serve(service: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, service).await.unwrap() });
    url
}

#[tokio::test]
async fn vllm_is_asked_the_way_the_python_front_asked() {
    let (url, seen) = fake_vllm((StatusCode::OK, MODEL_ANSWER.to_string())).await;
    let vllm = Vllm::new(&url, MODEL);
    let audio = Audio {
        bytes: b"abc".to_vec(),
        format: "mp3".into(),
    };

    let heard = vllm
        .transcribe(&audio, &Task::Context("Ada".into()), true)
        .await
        .unwrap();
    // The request body of `vllm_chat()` in front-py/speech_front.py.
    let mut want = json!({
        "model": MODEL, "temperature": 0, "max_tokens": 16384, "seed": 0,
        "messages": [{"role": "user", "content": [
            {"type": "text", "text": "Transcribe with punctuation and capitalization. Context terms: Ada."},
            {"type": "audio_url", "audio_url": {"url": "data:audio/mpeg;base64,YWJj"}}]}],
        "chat_template_kwargs": {"enable_thinking": false},
        "skip_special_tokens": false, "spaces_between_special_tokens": false,
        "mm_processor_kwargs": {"capture_ctc_timestamps": true},
    });
    assert_eq!(seen.lock().unwrap()[0], want);

    // The real model's answer, read.
    let word = |word: &str, start: f64, end: f64| Word {
        word: word.into(),
        start,
        end,
        speaker: 1,
    };
    assert_eq!(
        heard,
        Transcript {
            text: "Testing one to three.".into(),
            words: vec![
                word("Testing", 0.12, 0.605),
                word("one", 0.68, 0.842),
                word("to", 0.86, 0.986),
                word("three.", 1.0, 1.51)
            ],
            turns: vec![Turn {
                start: 0.0,
                end: 1.5,
                speaker: 1
            }],
            align_error: None,
        }
    );

    // Without word times, the request does not ask for them.
    vllm.transcribe(&audio, &Task::Context("Ada".into()), false)
        .await
        .unwrap();
    want.as_object_mut()
        .unwrap()
        .shift_remove("mm_processor_kwargs");
    assert_eq!(seen.lock().unwrap()[1], want);
}

#[tokio::test]
async fn vllm_dictation_requests_are_rewritten_for_the_model() {
    let (url, seen) = fake_vllm((
        StatusCode::OK,
        String::from_utf8(model_chat_answer()).unwrap(),
    ))
    .await;
    let app = app(Vllm::new(&url, MODEL), mock_speak());
    // What the Mac app sends: base64 of "abc" as the audio.
    let asked = json!({"model": "salm", "messages": [{"role": "user", "content": [
        {"type": "text", "text": "Vocabulary: Ada, Lumbridge."},
        {"type": "input_audio", "input_audio": {"data": "YWJj", "format": "wav"}}]}]});
    let a = post_json(&app, CHAT, &asked.to_string()).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(
        a.text(),
        OBSERVED_CHAT,
        "through the real adapter, still the observed answer"
    );
    assert_eq!(
        seen.lock().unwrap()[0],
        json!({"model": MODEL, "messages": [{"role": "user", "content": [
                   {"type": "text", "text": "Vocabulary: Ada, Lumbridge."},
                   {"type": "audio_url", "audio_url": {"url": "data:audio/wav;base64,YWJj"}}]}],
               "temperature": 0, "chat_template_kwargs": {"enable_thinking": false},
               "skip_special_tokens": false, "spaces_between_special_tokens": false})
    );

    let a = post_json(&app, CHAT, r#"{"messages":[{"role":"user","content":[{"type":"input_audio","input_audio":{"data":"***"}}]}]}"#).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        a.text(),
        r#"{"detail":"input_audio.data is not valid base64"}"#
    );
}

#[tokio::test]
async fn vllm_errors_and_other_paths() {
    let refusal = r#"{"error":{"message":"audio too long","type":"BadRequestError","param":null,"code":400}}"#;
    let (url, _) = fake_vllm((StatusCode::BAD_REQUEST, refusal.to_string())).await;
    let app = app(Vllm::new(&url, MODEL), mock_speak());
    assert!(app.transcriber.healthy().await);

    let a = upload(
        &app,
        TRANSCRIPTIONS,
        Some(("a.wav", &silence(16_000, 1600))),
        &[],
    )
    .await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        a.json(),
        json!({"detail": format!("transcribe: {refusal}")})
    );

    let a = get_path(&app, "/version").await;
    assert_eq!(
        (a.status, a.text()),
        (StatusCode::OK, r#"{"version":"0.32.0"}"#)
    );
    assert_eq!(
        get_path(&app, "/no/such/path").await.status,
        StatusCode::NOT_FOUND
    );

    let nowhere = Vllm::new("http://127.0.0.1:1", MODEL);
    assert!(!nowhere.healthy().await);
    let audio = Audio {
        bytes: vec![],
        format: "wav".into(),
    };
    assert!(matches!(
        nowhere.transcribe(&audio, &Task::Clean, false).await,
        Err(Failure::Unreachable(_))
    ));
}

#[tokio::test]
async fn with_real_adapters_ready_and_models_answer_as_observed() {
    let (vllm, _) = fake_vllm((StatusCode::OK, MODEL_ANSWER.to_string())).await;
    let magpie = fake_magpie().await;
    let app = app(Vllm::new(&vllm, MODEL), HttpSpeak::new(&magpie));
    assert_eq!(get_path(&app, "/ready").await.text(), OBSERVED_READY);
    assert_eq!(get_path(&app, "/v1/models").await.text(), OBSERVED_MODELS);

    let a = post_json(&app, "/v1/audio/speech", r#"{"input":"hi"}"#).await;
    assert_eq!(a.content_type.as_deref(), Some("audio/pcm"));
    assert_eq!(
        a.text(),
        r#"{"input":"hi"}"#,
        "the stand-in echoes what it was sent"
    );

    let nothing = crate::app(
        Vllm::new("http://127.0.0.1:1", MODEL),
        HttpSpeak::new("http://127.0.0.1:1"),
    );
    assert_eq!(
        get_path(&nothing, "/ready").await.text(),
        r#"{"capabilities":[],"device":"auto","ready":false,"transcribe":null,"tts":null}"#
    );
    assert_eq!(
        get_path(&nothing, "/v1/models").await.json()["data"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let a = post_json(&nothing, "/v1/audio/speech", "{}").await;
    assert_eq!(a.status, StatusCode::BAD_GATEWAY);
}

// ── Audio that needs ffmpeg ──────────────────────────────────────────────────
// Run with `cargo test -- --include-ignored` on a machine that has ffmpeg.

#[tokio::test]
#[ignore = "needs ffmpeg"]
async fn diarization_converts_other_audio_and_reports_its_real_length() {
    let app = app(model_that_heard_the_clip(), mock_speak());
    // The observed clip's format: 22050 Hz. This is the case where the Python front
    // reported 134217.7279375 seconds.
    let clip = silence(22_050, 34_816);
    let a = upload(
        &app,
        TRANSCRIPTIONS,
        Some(("tiny.wav", &clip)),
        &[("response_format", "verbose_json"), ("diarization", "true")],
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    let mut got = a.json();
    let duration = got
        .as_object_mut()
        .unwrap()
        .shift_remove("duration")
        .unwrap()
        .as_f64()
        .unwrap();
    assert!((duration - 1.579).abs() < 0.01, "duration {duration}");
    let mut want: Value = serde_json::from_str(OBSERVED_DIARIZED).unwrap();
    want.as_object_mut().unwrap().shift_remove("duration");
    assert_eq!(
        serde_json::to_string(&got).unwrap(),
        serde_json::to_string(&want).unwrap()
    );

    let calls = app.transcriber.calls.lock().unwrap().clone();
    let converted = wav::Wav::parse(&calls[0].audio.bytes).unwrap();
    assert_eq!(
        (converted.rate, converted.channels, converted.sample_width),
        (16_000, 1, 2)
    );

    let a = upload(
        &crate::app(MockTranscriber::default(), mock_speak()),
        TRANSCRIPTIONS,
        Some(("a.mp3", b"this is not audio")),
        &[("diarization", "true")],
    )
    .await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST);
    assert!(
        a.json()["detail"]
            .as_str()
            .unwrap()
            .starts_with("audio decode: "),
        "{}",
        a.text()
    );
}

// ── Against the real model ───────────────────────────────────────────────────
// Off unless LUMBRIDGE_VOICE_TEST_VLLM names a server that speaks the model's chat API
// and LUMBRIDGE_VOICE_TEST_CLIP is the clip the observed answers were made with.
// Six short requests. Nothing is started and nothing listens: requests go through
// the router in this process.

#[tokio::test]
#[ignore = "needs the real model: set LUMBRIDGE_VOICE_TEST_VLLM and LUMBRIDGE_VOICE_TEST_CLIP"]
async fn the_real_model_gives_the_observed_answers() {
    let (Ok(url), Ok(clip)) = (
        std::env::var("LUMBRIDGE_VOICE_TEST_VLLM"),
        std::env::var("LUMBRIDGE_VOICE_TEST_CLIP"),
    ) else {
        eprintln!("skipped: LUMBRIDGE_VOICE_TEST_VLLM and LUMBRIDGE_VOICE_TEST_CLIP are not set");
        return;
    };
    let clip = std::fs::read(clip).unwrap();
    let app = app(Vllm::new(&url, MODEL), MockSpeak::default());
    assert!(app.transcriber.healthy().await, "{url} is not healthy");

    // The clip says "one two three" and the model hears the middle word as "two" on
    // some runs and "to" on others, for the very same request (seen on the Python
    // front too, 2026-11-09). Everything else is compared exactly.
    let same = |got: &str, want: &str| {
        assert_eq!(got.replace(" two ", " to "), want.replace(" two ", " to "))
    };

    let a = upload(&app, TRANSCRIPTIONS, Some(("tiny.wav", &clip)), &[]).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    same(a.text(), r#"{"text":"Testing one to three."}"#);

    let a = upload(
        &app,
        TRANSCRIPTIONS,
        Some(("tiny.wav", &clip)),
        &[("response_format", "verbose_json"), ("language", "en")],
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    same(a.text(), OBSERVED_VERBOSE);

    let a = upload(&app, DIARIZATIONS, Some(("tiny.wav", &clip)), &[]).await;
    assert_eq!(
        (a.status, a.text()),
        (StatusCode::OK, OBSERVED_DIARIZATIONS)
    );

    // Who-said-what: everything as observed except the duration, which is now real.
    let a = upload(
        &app,
        TRANSCRIPTIONS,
        Some(("tiny.wav", &clip)),
        &[("response_format", "verbose_json"), ("diarization", "true")],
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    let mut got = a.json();
    let duration = got
        .as_object_mut()
        .unwrap()
        .shift_remove("duration")
        .unwrap()
        .as_f64()
        .unwrap();
    eprintln!(
        "real model, who-said-what: duration {duration} s (the Python front said 134217.7279375)"
    );
    assert!((duration - 1.579).abs() < 0.01, "duration {duration}");
    let mut want: Value = serde_json::from_str(OBSERVED_DIARIZED).unwrap();
    want.as_object_mut().unwrap().shift_remove("duration");
    same(
        &serde_json::to_string(&got).unwrap(),
        &serde_json::to_string(&want).unwrap(),
    );

    // Dictation, as the Mac app asks. The id, the time and the token counts are the
    // model's own; the rest is compared.
    let asked = json!({"model": "salm", "messages": [{"role": "user", "content": [
        {"type": "text", "text": "Transcribe with punctuation and capitalization."},
        {"type": "input_audio", "input_audio": {"data": base64_of(&clip), "format": "wav"}}]}]});
    let a = post_json(&app, CHAT, &asked.to_string()).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    let (got, want): (Value, Value) = (a.json(), serde_json::from_str(OBSERVED_CHAT).unwrap());
    for key in ["model", "object", "choices", "transcription"] {
        same(&got[key].to_string(), &want[key].to_string());
    }
    assert_eq!(
        got["transcription"],
        got["choices"][0]["message"]["content"]
    );
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(&got), keys(&want), "same fields in the same order");
    eprintln!(
        "real model: 5 answers as observed; dictation heard {}",
        got["transcription"]
    );
}

fn base64_of(bytes: &[u8]) -> String {
    // Through the crate's own helper, to avoid a test-only dependency.
    lumbridge_voice::text::data_url(bytes, "wav")
        .split_once(',')
        .unwrap()
        .1
        .to_string()
}

// ── Settings ─────────────────────────────────────────────────────────────────

#[test]
fn the_example_file_shows_the_defaults() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lumbridge-voice.example.toml");
    assert_eq!(
        Config::load(Some(&path), |_| None).unwrap(),
        Config::default()
    );
}
