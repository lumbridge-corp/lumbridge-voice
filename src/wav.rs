//! Just enough WAV: read the header to learn a recording's length, cut a piece out,
//! and turn any other audio into 16 kHz mono 16-bit WAV with `ffmpeg`.
//!
//! Reading follows Python's `wave` module, which the Python front used, with one
//! deliberate difference: the length is what is really in the file, not what the header
//! claims. `ffmpeg` writing to a pipe cannot fill in the header's length, and trusting
//! it made the Python front report a 1.6-second clip as 37 hours long.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// A WAV file's format and its samples.
pub struct Wav<'a> {
    pub channels: u16,
    /// Bytes per sample.
    pub sample_width: u16,
    pub rate: u32,
    /// The sample data really present in the file.
    pub data: &'a [u8],
}

impl<'a> Wav<'a> {
    /// `None` for anything Python's `wave` would refuse: not RIFF/WAVE, no `fmt ` chunk
    /// before the data, a format other than plain PCM.
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return None;
        }
        let u16_at = |i: usize| {
            bytes
                .get(i..i + 2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
        };
        let u32_at = |i: usize| {
            bytes
                .get(i..i + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let mut format: Option<(u16, u16, u32)> = None;
        let mut at = 12;
        loop {
            let name = bytes.get(at..at + 4)?;
            let size = u32_at(at + 4)? as usize;
            let body = at + 8;
            match name {
                b"fmt " => {
                    let tag = u16_at(body)?;
                    let channels = u16_at(body + 2)?;
                    let rate = u32_at(body + 4)?;
                    let bits = u16_at(body + 14)?;
                    let pcm = tag == 1 || (tag == 0xFFFE && u16_at(body + 24)? == 1);
                    let width = bits.div_ceil(8);
                    if !pcm || channels == 0 || width == 0 || rate == 0 {
                        return None;
                    }
                    format = Some((channels, width, rate));
                }
                b"data" => {
                    let (channels, sample_width, rate) = format?;
                    let end = body.saturating_add(size).min(bytes.len());
                    return Some(Wav {
                        channels,
                        sample_width,
                        rate,
                        data: &bytes[body.min(end)..end],
                    });
                }
                _ => {}
            }
            at = body.checked_add(size)?.checked_add(size & 1)?;
        }
    }

    fn frame_size(&self) -> usize {
        self.channels as usize * self.sample_width as usize
    }

    pub fn frames(&self) -> usize {
        self.data.len() / self.frame_size()
    }

    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / self.rate as f64
    }

    fn is_16k_mono_16bit(&self) -> bool {
        self.rate == 16_000 && self.channels == 1 && self.sample_width == 2
    }
}

/// Length of a WAV recording in seconds; `None` if it is not a WAV this can read.
pub fn seconds(raw: &[u8]) -> Option<f64> {
    Wav::parse(raw).map(|w| w.seconds())
}

/// A WAV file (44-byte header) around mono 16-bit samples.
pub fn write(pcm: &[u8], rate: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// The part of a WAV between two times, as a new WAV. Meant for 16 kHz mono 16-bit
/// input (what [`to_wav16k`] returns); the header written is always mono 16-bit.
pub fn slice(wav: &[u8], start: f64, end: f64) -> Option<Vec<u8>> {
    let w = Wav::parse(wav)?;
    let rate = w.rate as f64;
    let first = ((start * rate) as i64).max(0) as usize;
    let last = ((end * rate) as i64).max(0) as usize;
    let last = last.min(w.frames());
    let first = first.min(last);
    Some(write(
        &w.data[first * w.frame_size()..last * w.frame_size()],
        w.rate,
    ))
}

/// Why audio could not be converted: a status for the caller and a message.
pub type ConvertError = (u16, String);

/// Any audio as 16 kHz mono 16-bit WAV, so that turns can be cut out by sample. Audio
/// that already is comes back untouched.
pub async fn to_wav16k(ffmpeg: &str, raw: Vec<u8>) -> Result<Vec<u8>, ConvertError> {
    if Wav::parse(&raw).is_some_and(|w| w.is_16k_mono_16bit()) {
        return Ok(raw);
    }
    let mut child = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            "pipe:0",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-f",
            "wav",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| (500, format!("audio decode: cannot run {ffmpeg}: {e}")))?;
    let mut stdin = child.stdin.take().expect("stdin was piped");
    // Feed and drain at the same time, or a full pipe stalls both sides.
    let feed = tokio::spawn(async move {
        let _ = stdin.write_all(&raw).await;
        let _ = stdin.shutdown().await;
    });
    let done = tokio::time::timeout(Duration::from_secs(600), child.wait_with_output())
        .await
        .map_err(|_| {
            (
                500,
                "audio decode: ffmpeg timed out after 600 seconds".to_string(),
            )
        })?
        .map_err(|e| (500, format!("audio decode: {e}")))?;
    let _ = feed.await;
    if !done.status.success() {
        let shown = &done.stderr[..done.stderr.len().min(200)];
        return Err((
            400,
            format!("audio decode: {}", String::from_utf8_lossy(shown)),
        ));
    }
    Ok(done.stdout)
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use serde_json::Value;

    use super::*;

    fn cases() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/cases.json")).unwrap()
    }

    fn unb64(v: &Value) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(v.as_str().unwrap())
            .unwrap()
    }

    #[test]
    fn seconds_answers_as_the_python_front() {
        let cases = cases();
        let list = cases["audio_seconds"].as_array().unwrap();
        assert!(list.len() >= 11);
        for c in list {
            let name = c["wav"].as_str().unwrap();
            let got = seconds(&unb64(&cases["wavs"][name]));
            if name == "piped_lengths" {
                // The one deliberate difference: Python believed the header (134217.7 s).
                assert_eq!(c["out"].as_f64(), Some(134217.7279375));
                assert_eq!(got, Some(0.25));
            } else {
                assert_eq!(got, c["out"].as_f64(), "{name}");
            }
        }
    }

    #[test]
    fn slice_answers_as_the_python_front() {
        let cases = cases();
        let list = cases["wav_slice"].as_array().unwrap();
        assert!(list.len() >= 10);
        for c in list {
            let name = c["wav"].as_str().unwrap();
            let (start, end) = (c["start"].as_f64().unwrap(), c["end"].as_f64().unwrap());
            let got = slice(&unb64(&cases["wavs"][name]), start, end).unwrap();
            assert!(
                got == unb64(&c["out"]),
                "slice({name}, {start}, {end}): {} bytes",
                got.len()
            );
        }
    }

    #[tokio::test]
    async fn audio_already_16k_mono_is_not_touched() {
        let wav = write(&[1, 0, 2, 0], 16_000);
        // No such program: proof that ffmpeg was not asked.
        assert_eq!(
            to_wav16k("/nonexistent/ffmpeg", wav.clone()).await.unwrap(),
            wav
        );
        let other = write(&[1, 0, 2, 0], 22_050);
        assert_eq!(
            to_wav16k("/nonexistent/ffmpeg", other).await.unwrap_err().0,
            500
        );
    }
}
