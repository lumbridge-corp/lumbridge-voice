//! Small pure helpers shared by the front and the adapters. Each follows the function
//! of the same name in `front-py/speech_front.py`; `tests/fixtures/cases.json` holds
//! that file's own answers and the tests below compare against them.

use std::sync::LazyLock;

use base64::Engine;
use regex::Regex;
use serde_json::Value;

use crate::adapter::Turn;

static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<spk:\d+>|<\|[^|>]*\|>|</?s>").unwrap());
static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// Removes speaker tags and special tokens, then collapses whitespace.
pub fn clean(text: &str) -> String {
    let text = TAG.replace_all(text, " ");
    SPACE.replace_all(&text, " ").trim().to_string()
}

/// A speaker as callers see it: counted from 1. The model writes `<spk:0>`, `"0"`, `0`
/// or `spk_0` for the first speaker; anything without a number is speaker 1.
pub fn spk_int(tag: &Value) -> u32 {
    let text = match tag {
        Value::Null => return 1,
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let digits: String = text
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse::<u32>().map_or(1, |n| n + 1)
}

/// Who speaks a word: the turn that overlaps it most; if none overlaps, the nearest.
/// Turns come from the model's audio-based diarization, which separates voices more
/// reliably than the tags in its transcript.
pub fn speaker_at(turns: &[Turn], start: f64, end: f64) -> Option<u32> {
    let mut best: Option<&Turn> = None;
    let mut best_overlap = 0.0;
    for t in turns {
        let overlap = end.min(t.end) - start.max(t.start);
        if overlap > best_overlap {
            best = Some(t);
            best_overlap = overlap;
        }
    }
    if best.is_none() {
        let mid = (start + end) / 2.0;
        let mut nearest = f64::INFINITY;
        for t in turns {
            let distance = (mid - t.start).abs().min((mid - t.end).abs());
            if distance < nearest {
                best = Some(t);
                nearest = distance;
            }
        }
    }
    best.map(|t| t.speaker)
}

/// Media type for an audio format name or file extension; unknown ones count as WAV.
pub fn mime(format: &str) -> &'static str {
    match format.to_lowercase().as_str() {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "webm" => "audio/webm",
        "m4a" | "mp4" => "audio/mp4",
        _ => "audio/wav",
    }
}

/// The audio as a `data:` URL, which is how the model takes it.
pub fn data_url(raw: &[u8], format: &str) -> String {
    format!(
        "data:{};base64,{}",
        mime(format),
        base64::engine::general_purpose::STANDARD.encode(raw)
    )
}

/// Seconds rounded to milliseconds, as word times are reported.
pub fn round3(seconds: f64) -> f64 {
    (seconds * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cases() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/cases.json")).unwrap()
    }

    #[test]
    fn clean_answers_as_the_python_front() {
        let cases = cases();
        let list = cases["clean"].as_array().unwrap();
        assert!(list.len() >= 10);
        for c in list {
            let input = c["in"].as_str().unwrap_or(""); // one case is Python's None
            assert_eq!(clean(input), c["out"].as_str().unwrap(), "clean({input:?})");
        }
    }

    #[test]
    fn spk_int_answers_as_the_python_front() {
        let cases = cases();
        let list = cases["spk_int"].as_array().unwrap();
        assert!(list.len() >= 14);
        for c in list {
            assert_eq!(
                spk_int(&c["in"]) as u64,
                c["out"].as_u64().unwrap(),
                "spk_int({})",
                c["in"]
            );
        }
    }

    #[test]
    fn speaker_at_answers_as_the_python_front() {
        let cases = cases();
        let list = cases["speaker_at"].as_array().unwrap();
        assert!(list.len() >= 12);
        for c in list {
            let turns: Vec<Turn> = c["segs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| Turn {
                    start: s["start"].as_f64().unwrap(),
                    end: s["end"].as_f64().unwrap(),
                    speaker: spk_int(s.get("speaker").unwrap_or(&Value::Null)),
                })
                .collect();
            let (start, end) = (c["start"].as_f64().unwrap(), c["end"].as_f64().unwrap());
            assert_eq!(
                speaker_at(&turns, start, end).map(u64::from),
                c["out"].as_u64(),
                "speaker_at({start}, {end})"
            );
        }
    }

    #[test]
    fn data_url_answers_as_the_python_front() {
        let cases = cases();
        for c in cases["data_url"].as_array().unwrap() {
            let format = c["fmt"].as_str().unwrap();
            assert_eq!(
                data_url(b"abc", format),
                c["out"].as_str().unwrap(),
                "data_url(.., {format:?})"
            );
        }
    }

    #[test]
    fn round3_rounds_to_milliseconds() {
        assert_eq!(round3(0.12345), 0.123);
        assert_eq!(round3(3.35 + 0.2), 3.55);
        assert_eq!(round3(1.0), 1.0);
    }
}
