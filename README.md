# Lumbridge Voice

Speech to text: audio in, words out. A small HTTP front with an OpenAI-style API in front of a speech
model you run yourself, and a macOS dictation app (hold a key, talk, the words are typed). The front
holds no model: the model sits behind an adapter, so a new model is a new adapter, not a new API.

Part of [Lumbridge](https://lumbridgecorp.com) ([all repos](https://github.com/lumbridge-corp)). In daily use.

## Status

- **Version:** 0.1.0. Interfaces can change until 1.0.
- **Runs on:** the front on Linux and macOS (x86_64 and aarch64); the Mac app on macOS 14 or newer.
  The steps below are tested on Ubuntu 24.04, x86_64.
- **Not yet:**
  - No authentication on the front. Keep it on loopback or a private network (see [SECURITY.md](SECURITY.md)).
  - One model adapter: NVIDIA Nemotron 3.5 Transcribe on vLLM. You bring the model server.
  - No installer or download for the front or the Mac app: you build from source. The Mac app is not notarized.

## Quick start

You need git, a C toolchain and Rust 1.85 or newer. `ffmpeg` is used for who-said-what on audio that is
not already 16 kHz mono WAV.

On Ubuntu 24.04:

```sh
sudo apt-get update
sudo apt-get install -y git curl ca-certificates build-essential ffmpeg
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
. "$HOME/.cargo/env"
git clone https://github.com/lumbridge-corp/lumbridge-voice.git
cd lumbridge-voice
cargo build --release --locked
mkdir -p "$HOME/.local/bin"
install -m 755 target/release/lumbridge-voice "$HOME/.local/bin/"
```

On macOS, install the Xcode Command Line Tools (`xcode-select --install`), Rust from <https://rustup.rs>
and `ffmpeg` (for example with Homebrew), then build the same way.

The install is one binary, `~/.local/bin/lumbridge-voice`. It writes no files of its own. To remove it,
delete the binary.

Then try the front without a model: the `mock` adapter answers every request with "mock transcript".

```sh
LUMBRIDGE_VOICE_ADAPTER=mock "$HOME/.local/bin/lumbridge-voice" &
PID=$!
sleep 2
curl -s http://127.0.0.1:8816/health
printf 'RIFF....WAVEfmt ' > clip.wav
curl -s http://127.0.0.1:8816/v1/audio/transcriptions -F file=@clip.wav
kill "$PID"
```

You should see the readiness answer (`"ready":false` with `"tts":null`, because no text-to-speech server is
running), then `{"text":"mock transcript"}`.

## Point it at your own speech model server

For real transcription the front needs a speech model behind it. The adapter today is `vllm`: NVIDIA
Nemotron 3.5 Transcribe 30B-A4B served by vLLM with the NeMo Speech plugin, reached over vLLM's
OpenAI-compatible chat API. [`examples/serve-transcribe-vllm.sh`](examples/serve-transcribe-vllm.sh) starts
one on `127.0.0.1:8807`, following the model card (it needs an NVIDIA GPU that runs NVFP4).

Then tell the front where it is, in a settings file:

```toml
# voice.toml
listen = "127.0.0.1:8816"
adapter = "vllm"

[transcribe]
url = "http://127.0.0.1:8807"
model = "nvidia/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B-NVFP4"

# Optional: a text-to-speech server for POST /v1/audio/speech (NVIDIA Magpie TTS, for example).
[speak]
url = "http://127.0.0.1:8808"
```

```sh
lumbridge-voice --config voice.toml --print-config
lumbridge-voice --config voice.toml
```

The model server can be on another machine: set `transcribe.url` to its address.

## Configuration

Settings come from built-in defaults, then a TOML file (`--config FILE` or `LUMBRIDGE_VOICE_CONFIG`), then
the environment, which wins. [`lumbridge-voice.example.toml`](lumbridge-voice.example.toml) shows every
setting with its default.

| Setting | Environment | Default |
|---|---|---|
| `listen` | `LUMBRIDGE_VOICE_LISTEN` | `127.0.0.1:8816` |
| `adapter` | `LUMBRIDGE_VOICE_ADAPTER` | `vllm` (`mock` for no model) |
| `transcribe.url`, `transcribe.model` | `LUMBRIDGE_VOICE_TRANSCRIBE_URL`, `_TRANSCRIBE_MODEL` | `http://127.0.0.1:8807`, the Nemotron model id |
| `speak.url` | `LUMBRIDGE_VOICE_SPEAK_URL` | `http://127.0.0.1:8808` |
| `max_upload_mb`, `ffmpeg` | `LUMBRIDGE_VOICE_MAX_UPLOAD_MB`, `_FFMPEG` | `512`, `ffmpeg` |

## The API

[docs/API.md](docs/API.md) describes every route exactly. In short:

| Route | Purpose |
|---|---|
| `GET /ready`, `GET /health` | Readiness of the model and of text to speech |
| `GET /v1/models` | Model list |
| `POST /v1/chat/completions` | Dictation: audio plus instructions (the vocabulary goes here) |
| `POST /v1/audio/transcriptions` | Transcription; optional word times and who-said-what |
| `POST /v1/audio/diarizations` | Speaker turns only |
| `POST /v1/audio/translations` | Speech translated to another language |
| `POST /v1/audio/speech` | Text to speech, handed to the `speak.url` server |
| anything else | Handed to the model server unchanged |

## The Mac app

[`mac/`](mac/README.md) holds Lumbridge Voice for Mac: hold right Option to talk, tap it for hands-free,
and the words are typed into the frontmost app. It sends the audio to the front, keeps your vocabulary,
reads text aloud, and falls back to Apple's on-device recognizer when the front cannot be reached. Build
and install it on the Mac with `mac/install.sh`; set the front's address in its settings.

## Layout

- `src/`, `tests/`, `Cargo.toml` — the Rust front
- `front-py/` — the Python front the Rust one replaces, kept as the reference its tests compare against
- `examples/` — an example model server
- `vocab/` — an example shared vocabulary
- `mac/` — the Mac app (Swift), its command, build and install scripts
- `docs/` — the API, and a note on repeated digits

## How it fits the system

The front works alone; it needs only a model server. The Mac app needs the front (or any server with the
same API) and works offline with on-device speech.

| Works with | What passes between them |
|---|---|
| [lumbridge-speak](https://github.com/lumbridge-corp/lumbridge-speak) | Text to speech: the Mac app asks it to speak first; the front can hand `/v1/audio/speech` to it |

## What it sends and where

The front opens one HTTP port (loopback `127.0.0.1:8816` by default) and sends audio only to the model
server and text only to the text-to-speech server in its settings. It keeps nothing on disk: no audio, no
transcripts, no logs of content. It contacts no other service.

The Mac app records only while the key is held or hands-free is on, and sends the audio to the front you
set. It keeps a history of transcripts in `~/Library/Application Support/Lumbridge Voice/`. Ask sends a
screenshot and the question to the chat model you set (`brain`); `lumbridge-voice learn` sends candidate
words (from your history, your `~/.ssh/config` host names, `~/src` folder names and app names) to the same
model. Both go only to the `brain` address in its settings (loopback by default). It listens on port 8796
for spoken updates, from loopback and mesh VPN addresses, with a token. See [mac/README.md](mac/README.md).

## Docs

- [API](docs/API.md) — every route, field and status code
- [Repeated digits](docs/REPEATED-DIGITS.md) — why plain transcription is verbatim by default
- [Mac app](mac/README.md) — build, install, settings, the `lumbridge-voice` command
- [Change log](CHANGELOG.md)

## Contributing

Bug reports and small fixes are welcome. Talk to us in an issue before a large change.
See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test and send a pull request.

```sh
scripts/ci.sh
```

## Security

Do not open a public issue for a security problem.
Email security@lumbridgecorp.com or use "Report a vulnerability" on this repo's Security tab.
See [SECURITY.md](SECURITY.md).

## Licence

Apache-2.0. See [LICENSE](LICENSE).

Third-party code and assets keep their own licences. See [THIRD-PARTY-NOTICES](THIRD-PARTY-NOTICES).
Model weights are not part of this repo and have their own licences.
