# HTTP API of the speech front

This is the API of the speech front: the Rust front in `src/` and the Python front it
replaces, `front-py/speech_front.py`. It was written from the code, and the examples
marked **observed** are real responses of the Python front, in service on 2026-11-09,
made with a 1.6-second clip.

The front holds no model. It turns each request into a chat request for the Transcribe
model on vLLM (`TRANSCRIBE_URL`, default `http://127.0.0.1:8807`), and hands text to
speech to Magpie (`MAGPIE_URL`, default `http://127.0.0.1:8808`).

No authentication. No request size limit of its own. JSON responses are compact (no
spaces after `:` and `,`).

## Environment

| Variable | Default | Meaning |
|---|---|---|
| `TRANSCRIBE_URL` | `http://127.0.0.1:8807` | vLLM serving the Transcribe model |
| `MAGPIE_URL` | `http://127.0.0.1:8808` | Magpie text-to-speech server |
| `TRANSCRIBE_MODEL` | `nvidia/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B-NVFP4` | Model name sent to vLLM |

A deployment usually sets none of them. Host and port come from the uvicorn command
line (`0.0.0.0:8806`).

## Speaker numbers

Speakers are integers starting at **1** everywhere in this API. The model writes
`<spk:0>`, `<spk:1>`…; the front adds one.

## `GET /ready` and `GET /health`

Identical. Always status 200; look at `ready`.

The front asks vLLM `GET /health` (must be 200) and Magpie `GET /ready` (must have
`"ready": true`), each with a 4-second limit.

Observed:

```json
{"capabilities":["asr","diarization","speech-translation","tts"],"device":"auto","ready":true,"transcribe":"nvidia/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B-NVFP4","tts":"magpietts"}
```

- `capabilities`: `asr`, `diarization`, `speech-translation` when vLLM is healthy; `tts`
  when Magpie is ready.
- `ready`: true only when **both** are up.
- `transcribe`: the model name, or `null`. `tts`: `"magpietts"`, or `null`.

The model manager's health check looks for the text `"ready":true` in this body.

## `GET /v1/models`

Three fixed entries, then Magpie's own entry (the one with id `magpietts` from Magpie's
`/v1/models`) when Magpie answers. Observed:

```json
{"object":"list","data":[
 {"capability":"transcription","device":"auto","id":"salm","object":"model","owned_by":"local","speaker_diarization":true,"streaming":false,"word_timestamps":true,"root":"nvidia/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B-NVFP4"},
 {"capability":"diarization","device":"auto","id":"diarization","object":"model","owned_by":"local"},
 {"capability":"speech-translation","device":"auto","id":"speech-translation","object":"model","owned_by":"local"},
 {"capability":"speech","device":"auto","id":"magpietts","languages":["en-US","es-ES","de-DE","fr-FR","it-IT","vi-VN","hi-IN","ar-AE","ar-SA","ar-MSA","ko-KR","pt-BR"],"object":"model","owned_by":"local","voices":["John","Sofia","Aria","Jason","Leo"]}]}
```

## `POST /v1/chat/completions` (dictation)

Body: an OpenAI chat request in JSON. This is the route that accepts instructions, so it
is the only one where a vocabulary list reliably works.

What the front changes before sending it to vLLM:

- Every content part of type `input_audio` (`{"data": <base64>, "format": "wav"}`)
  becomes an `audio_url` part with a `data:` URL. `format` picks the media type: `wav`,
  `mp3`, `flac`, `ogg`, `webm`, `m4a`, `mp4`; anything else is sent as `audio/wav`.
- `model` is always replaced with the Transcribe model. Whatever the caller sent is
  ignored.
- Set only if the caller did not: `temperature: 0`,
  `chat_template_kwargs.enable_thinking: false`, `skip_special_tokens: false`,
  `spaces_between_special_tokens: false`.
- Everything else passes through.

Response when vLLM answers 200 and `stream` is not set: vLLM's response with three
changes:

- each `choices[].message.content` is cleaned (see [Cleaning](#cleaning));
- `model` is `"salm"`;
- a top-level `transcription` is added: the first choice's content (`""` if there are no
  choices).

Observed (shortened):

```json
{"id":"chatcmpl-…","object":"chat.completion","created":1794289515,"model":"salm",
 "choices":[{"index":0,"message":{"role":"assistant","content":"Testing one two three.", …},"finish_reason":"stop", …}],
 "usage":{"prompt_tokens":46,"total_tokens":52,"completion_tokens":6, …}, …,
 "transcription":"Testing one two three."}
```

Other cases:

- vLLM answers something other than 200: its body and status are returned as they are,
  as `application/json`. Observed with `{"messages":[]}`: status 400,
  `{"error":{"message":"Cannot apply chat template to an empty conversation. …","type":"BadRequestError","param":null,"code":400}}`.
- The same recording can come back with a word heard differently from one request to
  the next: the 1.6-second test clip ("one two three") gave "two" on some runs and "to"
  on others for the very same request (6 identical requests: 4 and 2). Callers must
  not expect identical text for identical audio.
- `stream: true`: the front waits for vLLM's whole event stream and returns it in one
  piece, with vLLM's status and content type. It is **not** cleaned and has no
  `transcription`. Nothing arrives before the model has finished.
- A body that is not JSON: status 500, plain text `Internal Server Error` (observed).

## `POST /v1/audio/transcriptions`

`multipart/form-data`.

| Field | Default | Meaning |
|---|---|---|
| `file` | required | The audio. The file name's extension picks the media type (same table as above). |
| `response_format` | `json` | `json`, `text` or `verbose_json`. Any other value behaves as `json`. |
| `diarization` | `false` | `1`, `true` or `yes` (any case) turns on who-said-what. |
| `prompt` | empty | Context terms. Used only when `diarization` is off. |
| `language` | empty | Echoed back in `verbose_json`. It does not steer the model. |
| `model` | `salm` | Ignored. |
| `timestamp_granularities` | empty | Ignored. |

Without `file`: status 422 (observed):

```json
{"detail":[{"type":"missing","loc":["body","file"],"msg":"Field required","input":null}]}
```

### Without diarization

One request to the model. The instruction is, with no `prompt`:

> Audio transcription. Language: as spoken. Wording: verbatim, including fillers and
> repetitions. Formatting: punctuation and capitalization. Output: transcript only.

and with the form field `wording=clean`, the instruction that was the default until
2026-11-10:

> Audio transcription. Language: as spoken. Wording: omit fillers and accidental
> repetitions; keep the remaining spoken wording unchanged. Formatting: punctuation and
> capitalization. Output: transcript only.

The clean instruction is no longer the default because it also drops digits that are
said twice or more in a row: of 25 clips of phone numbers, ports and codes, 4 came back
whole with it and 25 with the verbatim one (see `docs/REPEATED-DIGITS.md`). The per-turn
requests of a diarized transcription follow the same choice.

and with a `prompt`:

> Transcribe with punctuation and capitalization. Context terms: {prompt}.

Careful with `prompt`: the model may write the terms into the transcript. Observed with
`prompt="Ada, Lumbridge"` on a clip that says "Testing one two three":
`{"text":"Ada testing one to three Lumbridge."}`. For vocabulary, use
`/v1/chat/completions`.

Responses (observed):

- `json`: `{"text":"Testing one to three."}`
- `text`: the text alone, `text/plain; charset=utf-8`
- `verbose_json`:

```json
{"duration":1.5789569160997732,"language":"en","task":"transcribe","text":"Testing one to three.",
 "words":[{"confidence":0,"end":0.605,"speaker":1,"start":0.12,"word":"Testing"},
          {"confidence":0,"end":0.842,"speaker":1,"start":0.68,"word":"one"}, …]}
```

- `duration`: seconds, read from the WAV header of the upload. For anything that is not
  WAV: the end of the last word, or `0.0`.
- `words[].start`, `end`: seconds, rounded to 3 places. `confidence` is always `0`.
- `words[].speaker`: taken from the model's audio-based speaker turns (the turn that
  overlaps the word most, else the nearest), not from the tags in the text.
- Word times are requested from the model only for `verbose_json`.

### With diarization (`diarization=true`)

Built to survive the model under-tagging the second speaker:

1. The audio is converted to 16 kHz mono 16-bit WAV with `ffmpeg`, unless it already is.
2. One request for the whole recording to get speaker turns.
3. Neighbouring turns of the same speaker less than 1 second apart are merged.
4. Each turn is cut out (0.15 s of padding each side) and transcribed on its own with
   word times, all turns at once.
5. Word times are shifted back to the recording's clock.

`text` is the words joined with spaces. Responses: `json` and `text` as above;
`verbose_json` (observed):

```json
{"duration":134217.7279375,"language":"","task":"transcribe","text":"Testing one to three.",
 "words":[{"confidence":0,"start":0.12,"end":0.605,"speaker":1,"word":"Testing"}, …]}
```

Two things to know:

- The keys inside each word are in a **different order** here (`confidence, start, end,
  speaker, word`) than without diarization (`confidence, end, speaker, start, word`).
- **Known fault:** `duration` is wrong (`134217.7279375`, about 37 hours) whenever the
  upload was not already 16 kHz mono 16-bit WAV. `ffmpeg` writes to a pipe and cannot go
  back to fill in the WAV header's length, so the header says "maximum" and the front
  believes it. Words and times are right. The clip above was 22.05 kHz. Callers that
  always send 16 kHz mono WAV do not see this.

No speech found: `words` is `[]` and `text` is `""`.

## `POST /v1/audio/diarizations`

`multipart/form-data` with `file` (required; 422 as above without it). Speaker turns
only. Observed:

```json
{"segments":[{"end":1.5,"speaker":1,"start":0.0}]}
```

Times are not rounded.

## `POST /v1/audio/translations`

`multipart/form-data`: `file` (required), `language` (default `English`; the language to
translate **into**), `response_format` (`json` default, or `text`).

Instruction to the model: "Please translate the given speech to {language}."

Observed: `{"text":"Testing one to three."}`. With `text`: the text alone as plain text.

## `POST /v1/audio/speech`

Text to speech. The body and its content type go to Magpie's `/v1/audio/speech`
unchanged; Magpie's body, status and content type come back.

Magpie's contract, observed through this route:

- Body: `{"input": "...", "voice": "Leo", "language": "en-US", "response_format": "wav"}`.
  `model` is optional.
- `response_format: "wav"`: `audio/wav`, 22050 Hz mono 16-bit.
  `"pcm"`: `audio/pcm`, the same samples without a header.
- Voices: `John`, `Sofia`, `Aria`, `Jason`, `Leo`.
- Errors, status 400:
  `{"error":{"code":null,"message":"input is required","param":null,"type":"invalid_request_error"}}`,
  `… "message":"unknown voice_name 'Nobody'" …`.

## Any other path

`GET` or `POST` to any path not listed goes to vLLM at the same path, with the request
body and content type, and vLLM's answer comes back. The **query string is dropped**.
Other methods: status 405, `{"detail":"Method Not Allowed"}` (observed).

Observed: `GET /version` → `{"version":"0.32.0"}`; an unknown path →
404 `{"detail":"Not Found"}` (vLLM's own answer).

FastAPI's built-in pages (`/docs`, `/redoc`, `/openapi.json`) are also served.

## Errors

- A failure reported by this front: `{"detail":"<message>"}` with the status.
  - The model answered other than 200 on a transcription, diarization or translation:
    the model's status, `"transcribe: <first 300 characters of its answer>"`.
  - Word alignment failed: 502, `"ctc alignment: <reason>"`.
  - `ffmpeg` could not read the audio: 400, `"audio decode: <first 200 characters>"`.
- vLLM or Magpie cannot be reached at all: status 500, plain text
  `Internal Server Error`. (Read from the code; not provoked on the live service.)

## Cleaning

Transcript text has these removed, then runs of whitespace become one space and the ends
are trimmed: speaker tags `<spk:N>`, special tokens of the form `<|…|>`, and `<s>` /
`</s>`.

## What the front sends to the model

For transcription, diarization and translation, one chat request per call (or per turn):

```json
{"model": "<TRANSCRIBE_MODEL>", "temperature": 0, "max_tokens": 16384, "seed": 0,
 "messages": [{"role": "user", "content": [
   {"type": "text", "text": "<instruction>"},
   {"type": "audio_url", "audio_url": {"url": "data:audio/wav;base64,…"}}]}],
 "chat_template_kwargs": {"enable_thinking": false},
 "skip_special_tokens": false, "spaces_between_special_tokens": false,
 "mm_processor_kwargs": {"capture_ctc_timestamps": true}}
```

`mm_processor_kwargs` is present only when word times or speaker turns are wanted. The
model's answer then carries a top-level `ctc_timestamps` object with `words`
(`word`, `start`, `end`, `speaker`), `diarization` (`start`, `end`, `speaker`) and, on
failure, `error`.

Time limits: 3600 seconds per request to the model, 5 seconds to connect.

## Differences in the Rust front

The Rust front (`src/`) answers the same as the Python front for everything above, with
these exceptions.

| Case | Python front | Rust front |
|---|---|---|
| `duration` with `diarization=true` when the upload is not 16 kHz mono 16-bit WAV | Wrong: `134217.7279375` (the known fault above) | The real length. Checked against the real model: `1.579` for the 1.58-second clip. |
| `duration` of a WAV whose header claims more audio than the file holds | What the header claims | What the file holds |
| The model server cannot be reached | 500, plain text `Internal Server Error` | 502, `{"detail":"transcribe: unreachable: <reason>"}` (for `/v1/audio/speech`: `speech: unreachable: …`) |
| `POST /v1/chat/completions` with a body that is not JSON, or not a JSON object | 500, plain text `Internal Server Error` | 400, `{"detail":"the request body must be JSON"}` |
| `input_audio.data` that is not valid base64 | 500 for wrong padding; stray characters are skipped silently | 400, `{"detail":"input_audio.data is not valid base64"}` |
| The model's 200 answer to a chat request is not a JSON object | 500 | 502 with a `detail` |
| Request body size | No limit | `max_upload_mb`, default 512; larger → 413 |
| `/docs`, `/redoc`, `/openapi.json` | FastAPI's built-in pages | Not there; like any other path they go to the model server (vLLM has its own) |
| A `file` field sent as plain text, without a file name | 422 | Taken as the audio |
| Word times that fall exactly on half a millisecond | Python's `round()` | Rounded half away from zero; can differ by 0.001 s |
| Very small or very large numbers inside a passed-through chat answer | Python's notation (`1e-05`) | Rust's (`1e-5`); the same number |
| `Server` header | `uvicorn` | none |
| A path with percent-encoded characters handed to the model server | Decoded, then encoded again | Passed as received |
| Listens on | all interfaces, port 8806 (from the command line) | loopback, port 8816, until settings say otherwise |

Not different, on purpose: the query string is still dropped when a request is handed
to the model server, `stream: true` is still answered in one piece, and `/ready` still
needs text to speech to be up before it says `ready`.
