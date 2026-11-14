//! Settings: built-in defaults, then a TOML file, then the environment
//! (`LUMBRIDGE_VOICE_*`). The defaults expect the model servers on
//! this machine's loopback; the front itself listens on loopback port 8816.

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Address and port to listen on.
    pub listen: String,
    /// Which model adapter: `vllm` or `mock`.
    pub adapter: String,
    /// Largest request body accepted, in megabytes. Recordings are uploaded whole.
    pub max_upload_mb: usize,
    /// The `ffmpeg` program, used for who-said-what on audio that is not 16 kHz mono WAV.
    pub ffmpeg: String,
    pub transcribe: Transcribe,
    pub speak: Speak,
}

/// The speech model (adapter `vllm`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Transcribe {
    pub url: String,
    pub model: String,
}

/// The text-to-speech server that `/v1/audio/speech` is handed to.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Speak {
    pub url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8816".into(),
            adapter: "vllm".into(),
            max_upload_mb: 512,
            ffmpeg: "ffmpeg".into(),
            transcribe: Transcribe::default(),
            speak: Speak::default(),
        }
    }
}

impl Default for Transcribe {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:8807".into(),
            model: "nvidia/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B-NVFP4".into(),
        }
    }
}

impl Default for Speak {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:8808".into(),
        }
    }
}

impl Config {
    /// `file` is the TOML file, if any. `env` looks up an environment variable; an empty
    /// value counts as not set.
    pub fn load(file: Option<&Path>, env: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let mut c = match file {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?
            }
            None => Config::default(),
        };
        let get = |name: &str| env(&format!("LUMBRIDGE_VOICE_{name}")).filter(|v| !v.is_empty());
        if let Some(v) = get("LISTEN") {
            c.listen = v;
        }
        if let Some(v) = get("ADAPTER") {
            c.adapter = v;
        }
        if let Some(v) = get("MAX_UPLOAD_MB") {
            c.max_upload_mb = v
                .parse()
                .map_err(|_| format!("LUMBRIDGE_VOICE_MAX_UPLOAD_MB: not a number: {v}"))?;
        }
        if let Some(v) = get("FFMPEG") {
            c.ffmpeg = v;
        }
        if let Some(v) = get("TRANSCRIBE_URL") {
            c.transcribe.url = v;
        }
        if let Some(v) = get("TRANSCRIBE_MODEL") {
            c.transcribe.model = v;
        }
        if let Some(v) = get("SPEAK_URL") {
            c.speak.url = v;
        }
        if !matches!(c.adapter.as_str(), "vllm" | "mock") {
            return Err(format!("adapter must be vllm or mock, not {}", c.adapter));
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_live_values_on_another_port() {
        let c = Config::load(None, |_| None).unwrap();
        assert_eq!(c.listen, "127.0.0.1:8816");
        assert_eq!(c.adapter, "vllm");
        assert_eq!(c.transcribe.url, "http://127.0.0.1:8807");
        assert_eq!(
            c.transcribe.model,
            "nvidia/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B-NVFP4"
        );
        assert_eq!(c.speak.url, "http://127.0.0.1:8808");
    }

    #[test]
    fn file_then_environment() {
        let dir = std::env::temp_dir().join(format!("lumbridge-voice-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("voice.toml");
        std::fs::write(
            &path,
            "listen = \"0.0.0.0:8806\"\nmax_upload_mb = 64\n[transcribe]\nmodel = \"other\"\n",
        )
        .unwrap();
        let env = |name: &str| match name {
            "LUMBRIDGE_VOICE_TRANSCRIBE_MODEL" => Some("from-env".to_string()),
            "LUMBRIDGE_VOICE_SPEAK_URL" => Some(String::new()), // empty = not set
            _ => None,
        };
        let c = Config::load(Some(&path), env).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(c.listen, "0.0.0.0:8806"); // from the file
        assert_eq!(c.max_upload_mb, 64);
        assert_eq!(c.transcribe.model, "from-env"); // the environment wins over the file
        assert_eq!(c.transcribe.url, "http://127.0.0.1:8807"); // default kept
        assert_eq!(c.speak.url, "http://127.0.0.1:8808");
    }

    #[test]
    fn mistakes_are_reported() {
        let env = |name: &str| (name == "LUMBRIDGE_VOICE_ADAPTER").then(|| "whisper".to_string());
        assert!(Config::load(None, env)
            .unwrap_err()
            .contains("vllm or mock"));
        let env = |name: &str| (name == "LUMBRIDGE_VOICE_MAX_UPLOAD_MB").then(|| "big".to_string());
        assert!(Config::load(None, env)
            .unwrap_err()
            .contains("not a number"));
    }
}
