//! The models behind the front.
//!
//! The front knows the HTTP API: form fields, response shapes, who-said-what by turns.
//! A [`Transcriber`] knows one speech model: how to ask it for a transcript, word times
//! and speaker turns. Adding a model means adding an adapter (see [`crate::vllm`]) and
//! a name in [`crate::config`]; the front does not change.
//!
//! [`Speak`] is the text-to-speech server the front hands `/v1/audio/speech` to. That
//! route is kept because callers use it; it belongs to lumbridge-speak.

use std::future::Future;
use std::sync::Mutex;

use serde_json::{json, Value};

/// What to ask the model to do with a piece of audio.
#[derive(Debug, Clone, PartialEq)]
pub enum Task {
    /// The words as spoken: punctuation, fillers and repetitions kept. The default: the
    /// clean instruction also drops digits said twice or more in a row ("five five five").
    Verbatim,
    /// A clean transcript: punctuation, no fillers, no repetitions. Only on request.
    Clean,
    /// A transcript, given terms that may occur in it.
    Context(String),
    /// A verbatim transcript with speakers marked.
    Speakers,
    /// A translation into the named language.
    Translate(String),
}

/// A piece of audio and its format name (`wav`, `mp3`, …; usually the file extension).
#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    pub bytes: Vec<u8>,
    pub format: String,
}

/// One word with its place in the audio. `speaker` counts from 1.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub word: String,
    pub start: f64,
    pub end: f64,
    pub speaker: u32,
}

/// A stretch of audio in which one speaker talks. `speaker` counts from 1.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub start: f64,
    pub end: f64,
    pub speaker: u32,
}

/// What the model made of a piece of audio.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Transcript {
    /// As the model wrote it; may still carry speaker tags and special tokens.
    pub text: String,
    /// Empty unless times were asked for.
    pub words: Vec<Word>,
    /// Speaker turns heard in the audio. Empty unless times were asked for.
    pub turns: Vec<Turn>,
    /// Set when times were asked for and the model could not align them.
    pub align_error: Option<String>,
}

/// An HTTP answer passed along as it came.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// Why a model could not answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    /// It answered, with this status and text.
    Status { status: u16, text: String },
    /// It could not be reached.
    Unreachable(String),
    /// The caller's request cannot be passed on.
    BadRequest(String),
}

/// A speech-to-text model.
pub trait Transcriber: Send + Sync + 'static {
    /// The model's name, reported by `/ready` and `/v1/models`.
    fn model(&self) -> &str;
    fn healthy(&self) -> impl Future<Output = bool> + Send;
    /// One pass over one piece of audio. With `timestamps`, also word times and turns.
    fn transcribe(
        &self,
        audio: &Audio,
        task: &Task,
        timestamps: bool,
    ) -> impl Future<Output = Result<Transcript, Failure>> + Send;
    /// A dictation client's OpenAI chat request (audio as `input_audio` parts), answered
    /// in OpenAI's shape. The front tidies the answer; the adapter only gets it.
    fn chat(&self, body: Value) -> impl Future<Output = Result<Reply, Failure>> + Send;
    /// Any other request, handed to the model server as it is.
    fn forward(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> impl Future<Output = Result<Reply, Failure>> + Send;
}

/// The text-to-speech server behind `/v1/audio/speech`.
pub trait Speak: Send + Sync + 'static {
    fn ready(&self) -> impl Future<Output = bool> + Send;
    /// The entries of its model list.
    fn models(&self) -> impl Future<Output = Vec<Value>> + Send;
    /// A speech request, handed over as it is.
    fn speech(
        &self,
        content_type: &str,
        body: Vec<u8>,
    ) -> impl Future<Output = Result<Reply, Failure>> + Send;
}

/// One call the mock model received.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub audio: Audio,
    pub task: Task,
    pub timestamps: bool,
}

/// `(method, path, content type, body)` of a request the mock model was handed.
pub type Forwarded = (String, String, String, Vec<u8>);

/// A stand-in model for tests and for trying the front without a GPU. It answers every
/// piece of audio with the same transcript and remembers what it was asked.
pub struct MockTranscriber {
    pub model: String,
    pub healthy: bool,
    /// The answer to every `transcribe`. Words and turns are left out when times are
    /// not asked for.
    pub transcript: Transcript,
    /// When set, `transcribe`, `chat` and `forward` fail with it.
    pub fail: Option<Failure>,
    pub chat_reply: Reply,
    pub forward_reply: Reply,
    pub calls: Mutex<Vec<Call>>,
    pub chats: Mutex<Vec<Value>>,
    pub forwards: Mutex<Vec<Forwarded>>,
}

impl Default for MockTranscriber {
    fn default() -> Self {
        let json = |v: Value| Reply {
            status: 200,
            content_type: Some("application/json".into()),
            body: v.to_string().into_bytes(),
        };
        Self {
            model: "mock".into(),
            healthy: true,
            transcript: Transcript {
                text: "mock transcript".into(),
                ..Default::default()
            },
            fail: None,
            chat_reply: json(
                json!({"model": "mock", "choices": [{"index": 0, "message": {"role": "assistant", "content": "mock transcript"}}]}),
            ),
            forward_reply: json(json!({"mock": true})),
            calls: Mutex::default(),
            chats: Mutex::default(),
            forwards: Mutex::default(),
        }
    }
}

impl Transcriber for MockTranscriber {
    fn model(&self) -> &str {
        &self.model
    }

    async fn healthy(&self) -> bool {
        self.healthy
    }

    async fn transcribe(
        &self,
        audio: &Audio,
        task: &Task,
        timestamps: bool,
    ) -> Result<Transcript, Failure> {
        self.calls.lock().unwrap().push(Call {
            audio: audio.clone(),
            task: task.clone(),
            timestamps,
        });
        if let Some(f) = &self.fail {
            return Err(f.clone());
        }
        let mut t = self.transcript.clone();
        if !timestamps {
            t.words.clear();
            t.turns.clear();
            t.align_error = None;
        }
        Ok(t)
    }

    async fn chat(&self, body: Value) -> Result<Reply, Failure> {
        self.chats.lock().unwrap().push(body);
        self.fail.clone().map_or(Ok(self.chat_reply.clone()), Err)
    }

    async fn forward(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<Reply, Failure> {
        self.forwards
            .lock()
            .unwrap()
            .push((method.into(), path.into(), content_type.into(), body));
        self.fail
            .clone()
            .map_or(Ok(self.forward_reply.clone()), Err)
    }
}

/// A stand-in text-to-speech server.
pub struct MockSpeak {
    pub ready: bool,
    pub models: Vec<Value>,
    pub reply: Result<Reply, Failure>,
    /// `(content type, body)` of every speech request.
    pub calls: Mutex<Vec<(String, Vec<u8>)>>,
}

impl Default for MockSpeak {
    fn default() -> Self {
        Self {
            ready: true,
            models: Vec::new(),
            reply: Ok(Reply {
                status: 200,
                content_type: Some("audio/wav".into()),
                body: crate::wav::write(&[0; 32], 22_050),
            }),
            calls: Mutex::default(),
        }
    }
}

impl Speak for MockSpeak {
    async fn ready(&self) -> bool {
        self.ready
    }

    async fn models(&self) -> Vec<Value> {
        self.models.clone()
    }

    async fn speech(&self, content_type: &str, body: Vec<u8>) -> Result<Reply, Failure> {
        self.calls.lock().unwrap().push((content_type.into(), body));
        self.reply.clone()
    }
}
