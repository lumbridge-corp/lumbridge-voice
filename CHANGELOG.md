# Changelog

## [Unreleased]

## [0.1.0] - 2026-11-13

The first release of Lumbridge Voice. It is built from source.

- The speech-to-text front (`lumbridge-voice`, Rust): the OpenAI-style routes for dictation, transcription
  with word times and who-said-what, speaker turns and translation, with the model behind an adapter.
  Adapters: `vllm` (NVIDIA Nemotron 3.5 Transcribe on vLLM) and `mock`. Listens on loopback port 8816 by
  default. No authentication.
- Plain and diarized transcription are verbatim by default, so digits said more than once are kept;
  `wording=clean` drops fillers (docs/REPEATED-DIGITS.md).
- `/v1/audio/speech` is handed to a text-to-speech server.
- Lumbridge Voice for Mac: right Option dictation, read-aloud, Ask, spoken updates, and the
  `lumbridge-voice` command. Every server address is a setting; the defaults are on loopback.
- An example model server (`examples/serve-transcribe-vllm.sh`) and an example shared vocabulary.
- `scripts/scrub-check` fails on private addresses, home folders, email addresses and token formats in the tree.
