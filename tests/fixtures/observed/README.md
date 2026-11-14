# Observed responses

Real response bodies of the Python front (`front-py/`), saved byte for byte on
2026-11-09. The clip was 1.58 seconds of synthetic speech ("Testing one two three."),
22050 Hz mono 16-bit WAV, made by the text-to-speech front. The clip itself is not in
the repo (no audio is).

| File | Request |
|---|---|
| `ready.json` | `GET /ready` |
| `models.json` | `GET /v1/models` |
| `transcription-verbose.json` | `POST /v1/audio/transcriptions` with `response_format=verbose_json`, `language=en` |
| `transcription-diarized-verbose.json` | the same with `diarization=true` and no `language`. Its `duration` shows the fault described in `docs/API.md`. |
| `diarizations.json` | `POST /v1/audio/diarizations` |
| `chat.json` | `POST /v1/chat/completions` with an `input_audio` part and the instruction "Transcribe with punctuation and capitalization." |
| `chat-with-word-times.json` | `POST /v1/chat/completions` with the speakers instruction and `mm_processor_kwargs.capture_ctc_timestamps`. The front passes that through, so this shows the model's own `ctc_timestamps` object. |

The Rust tests feed the mock model what the real model said and require the Rust front
to answer with exactly these bytes.
