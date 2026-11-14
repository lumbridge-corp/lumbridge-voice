# Lumbridge Voice for Mac

Dictation and a small assistant pill for macOS 14 or newer.

- **Right Option:** tap for hands-free dictation, hold for push-to-talk; an Option chord cancels. The audio
  goes to the speech front (`server`, chat API) with your vocabulary, and falls back to Apple's on-device
  speech when the front cannot be reached. The text is typed into the frontmost app.
- **⌃⌥V** pastes the last dictation, **⌃⌥S** reads the selected text aloud.
- **Pill (bottom right):** dictate, Ask (a screenshot plus your spoken question to the chat model in
  `brain`, answered aloud), region screenshot, read the selection aloud, recent transcriptions.
- **Spoken updates:** an announce server on `:8796` (`POST /say` with a Bearer token) that agents and
  scripts can use to speak through this Mac.
- **`lumbridge-voice` command:** `status`, `last`, `add`, `fix`, `forget`, `learn` (nightly 03:40),
  `vocab`, `say`, `voice`, `sync-files`. `lumbridge-voice --help` lists them.

## Files

| File | What it is |
|---|---|
| `main.swift` | The key, recording, phrase cutting, the calls to the front, paste, vocabulary and fixes, history, the speaker, the announce server, Ask, the menu |
| `pill.swift` | The floating pill and its read-along text |
| `lumbridge-voice` | The command (Python 3, standard library only) |
| `Info.plist`, `fonts/` | Bundle. The fonts are Geist and Geist Mono, under the SIL Open Font License (`fonts/OFL.txt`) |
| `com.lumbridgecorp.lumbridge-voice*.plist` | Three LaunchAgents: the app, the nightly `learn`, the screenshot upload. `__HOME__` is filled in by `install.sh` |
| `build.sh` | Builds into `mac/build/`. Installs nothing |
| `install.sh` | Builds, installs, loads the agents |

## Build

You need the Xcode Command Line Tools (`xcode-select --install`).

```sh
mac/build.sh
```

Compiles the two Swift files into `mac/build/Lumbridge Voice.app` and signs it ad-hoc with
`designated => identifier "<bundle id>"`, so macOS sees a rebuild as the same app and keeps its permissions.
It writes nothing outside `mac/build/`. `LUMBRIDGE_VOICE_SIGN=dev mac/build.sh` signs with the Apple
Development identity in your keychain instead. The bundle id is `com.lumbridgecorp.lumbridge-voice.app`;
`LUMBRIDGE_VOICE_BUNDLE_ID` sets another (give the same value to `install.sh`).

## Install

```sh
mac/install.sh
```

Run it on the Mac, as yourself, when you are not dictating. It stops without changing anything if a
dictation or read-aloud is in progress, if the build is missing or wrongly signed, or if a name it needs
belongs to something else. It installs `/Applications/Lumbridge Voice.app`, `~/.local/bin/lumbridge-voice`
and three LaunchAgents in `~/Library/LaunchAgents`, and loads them. Logs go to
`~/Library/Logs/Lumbridge Voice/`. Running it again is safe.

On first start macOS asks for Microphone, Accessibility and Input Monitoring (and Screen Recording for
Ask). Allow "Lumbridge Voice" in System Settings > Privacy & Security.

To remove it: `launchctl bootout gui/$(id -u)/<label>` for the three labels, then delete the app, the
command, the three agent files and `~/Library/Application Support/Lumbridge Voice`.

## Settings

`~/Library/Application Support/Lumbridge Voice/voice.json`. All keys are optional. Example:

```json
{
  "server": "http://127.0.0.1:8816",
  "brain": "http://127.0.0.1:8000/v1/chat/completions",
  "brain_model": "my-vision-model"
}
```

| Key | Default | Meaning |
|---|---|---|
| `server` | `http://127.0.0.1:8816` | The speech front (speech to text, and Magpie text to speech as the fallback voice). `LUMBRIDGE_VOICE_SERVER` overrides it |
| `speak` | `http://127.0.0.1:8813` | [Lumbridge Speak](https://github.com/lumbridge-corp/lumbridge-speak) on this Mac: everything spoken is asked for here first. If it does not answer, the app asks Magpie through `server`. `LUMBRIDGE_VOICE_SPEAK` overrides it |
| `brain` | `http://127.0.0.1:8000/v1/chat/completions` | An OpenAI-compatible chat model (with vision) for Ask and `learn`. `LUMBRIDGE_VOICE_BRAIN` overrides it |
| `brain_model` | `brain` | The model name sent to `brain` |
| `fleet_vocab` | none | Where `learn` copies a shared vocabulary from: an http(s) URL or an scp source (`host:path`). The format is [`vocab/boosting.example.json`](../vocab/boosting.example.json) |
| `files_api` | none | Where `sync-files` uploads new screenshots (`POST <files_api>?id=..&name=..`, Bearer token from `files.token`). Unset: nothing is uploaded |
| `voice`, `rate`, `updates_voice`, `updates_rate` | `Leo`, `1.0`, `Leo`, `1.1` | Voices and speeds |
| `announce` | `true` | Speak announcements |
| `pill`, `pill_offset`, `theme` | `true`, none, `dark` | The pill |

The server addresses are read when the app starts; the menu's other settings apply at once.

Your vocabulary is `vocab.json` in the same folder (`lumbridge-voice add`, `fix`, `forget`, or "Edit my
vocabulary" in the menu). It can also hold `"speaker": "<your name>"`, which is put in the model's
instructions, and `"style"`, a sentence of extra instructions.

## What it sends and where

- Audio, only while you dictate, to `server`. On-device speech when that fails.
- Ask: one screenshot (resized, never stored) and the question, to `brain`.
- `learn`: candidate words from your dictation history, `~/.ssh/config` host names, `~/src` folder names
  and application names, to `brain`.
- `sync-files`: new screenshots, only when `files_api` is set.
- The announce server on port 8796 accepts connections from loopback and from mesh VPN addresses
  (100.64.0.0/10 and Tailscale's IPv6 prefix) only, and needs the token in `announce.token` (made on first
  start, mode 0600). `GET /health` needs no token.

Data (history, vocabulary, tokens) lives in the support folder and is never committed.
