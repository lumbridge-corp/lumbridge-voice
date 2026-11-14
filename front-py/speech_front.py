"""speech-front (:8806) — keeps the NeMo-Speech.cpp :8806 contract while transcription runs on NVIDIA's
official Nemotron 3.5 Transcribe NVFP4 (vLLM 0.32 + NeMo Speech, :8807) and TTS on speech-magpie (:8808).
2026-11-09. Callers unchanged: Mac dictation (/v1/chat/completions + input_audio), older who-said-what
contracts (/v1/audio/transcriptions diarization=true, /v1/audio/diarizations), TTS (/v1/audio/speech).

Request formats follow the model card: audio_url data URLs, skip_special_tokens=False, enable_thinking=False,
word timestamps via mm_processor_kwargs.capture_ctc_timestamps (needs the ctc_timestamp middleware).
"""
import base64, logging, os, re

import httpx
from fastapi import FastAPI, File, Form, HTTPException, Request, UploadFile
from fastapi.responses import JSONResponse, PlainTextResponse, Response

VLLM = os.environ.get("TRANSCRIBE_URL", "http://127.0.0.1:8807")
MAGPIE = os.environ.get("MAGPIE_URL", "http://127.0.0.1:8808")
MODEL = os.environ.get("TRANSCRIBE_MODEL", "nvidia/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B-NVFP4")
log = logging.getLogger("uvicorn.error")
client = httpx.AsyncClient(timeout=httpx.Timeout(3600, connect=5))
app = FastAPI(title="speech-front")

# Model-card prompts (Prompt Recommendations table).
P_CLEAN = ("Audio transcription. Language: as spoken. Wording: omit fillers and accidental repetitions; keep the "
           "remaining spoken wording unchanged. Formatting: punctuation and capitalization. Output: transcript only.")
# The default. P_CLEAN also drops digits said twice or more in a row ("five five five" -> "five five"): 4 of 25
# phone numbers and codes came back whole with it, 25 of 25 with this one (2026-11-10). Clean is `wording=clean`.
P_VERBATIM = ("Audio transcription. Language: as spoken. Wording: verbatim, including fillers and repetitions. "
              "Formatting: punctuation and capitalization. Output: transcript only.")
P_SPEAKERS = ("Transcribe this audio with punctuation and capitalization. Include all spoken words and verbal fillers. "
              "Write numbers in their standard written form. Put speaker tags <spk:0> <spk:1> <spk:2> <spk:3> on the transcription.")
P_CONTEXT = "Transcribe with punctuation and capitalization. Context terms: {terms}."
P_TRANSLATE = "Please translate the given speech to {lang}."
TAG = re.compile(r"<spk:\d+>|<\|[^|>]*\|>|</?s>")
MIME = {"wav": "audio/wav", "mp3": "audio/mpeg", "flac": "audio/flac", "ogg": "audio/ogg", "webm": "audio/webm",
        "m4a": "audio/mp4", "mp4": "audio/mp4"}


def data_url(raw: bytes, fmt: str = "wav") -> str:
    return f"data:{MIME.get(fmt.lower(), 'audio/wav')};base64,{base64.b64encode(raw).decode()}"


def clean(text: str) -> str:
    return re.sub(r"\s+", " ", TAG.sub(" ", text or "")).strip()


async def vllm_chat(prompt: str, audio_url: str, timestamps: bool = False, **extra) -> dict:
    body = {"model": MODEL, "temperature": 0, "max_tokens": 16384, "seed": 0,
            "messages": [{"role": "user", "content": [{"type": "text", "text": prompt},
                                                       {"type": "audio_url", "audio_url": {"url": audio_url}}]}],
            "chat_template_kwargs": {"enable_thinking": False},
            "skip_special_tokens": False, "spaces_between_special_tokens": False}
    if timestamps:
        body["mm_processor_kwargs"] = {"capture_ctc_timestamps": True}
    body.update(extra)
    r = await client.post(f"{VLLM}/v1/chat/completions", json=body)
    if r.status_code != 200:
        raise HTTPException(r.status_code, f"transcribe: {r.text[:300]}")
    return r.json()


def spk_int(tag) -> int:
    """'<spk:0>' / 'spk_0' / 0 -> 1-based int, like NeMo-Speech.cpp."""
    m = re.search(r"\d+", str(tag if tag is not None else 0))
    return int(m.group()) + 1 if m else 1


def speaker_at(segs: list, start: float, end: float) -> int | None:
    """Word speaker from the audio-derived diarization (like NeMo-Speech.cpp did), not from the transcript's
    <spk:N> tags: the model often tags a whole transcript <spk:0> while diarization separates the voices.
    Speaker of the segment with the most overlap; if none overlaps, the nearest segment. 1-based."""
    if not segs:
        return None
    best, best_ov = None, 0.0
    for s in segs:
        ov = min(end, float(s["end"])) - max(start, float(s["start"]))
        if ov > best_ov:
            best, best_ov = s, ov
    if best is None:
        mid = (start + end) / 2
        best = min(segs, key=lambda s: min(abs(mid - float(s["start"])), abs(mid - float(s["end"]))))
    return spk_int(best.get("speaker"))


def ctc(resp: dict) -> dict:
    a = resp.get("ctc_timestamps") or {}
    if a.get("error"):
        raise HTTPException(502, f"ctc alignment: {a['error']}")
    return a


def audio_seconds(raw: bytes) -> float | None:
    if raw[:4] == b"RIFF":
        import io, wave
        try:
            w = wave.open(io.BytesIO(raw)); return w.getnframes() / float(w.getframerate())
        except Exception:
            return None
    return None


@app.get("/ready")
async def ready():
    ok_t = ok_m = False
    try:
        ok_t = (await client.get(f"{VLLM}/health", timeout=4)).status_code == 200
    except Exception:
        pass
    try:
        ok_m = (await client.get(f"{MAGPIE}/ready", timeout=4)).json().get("ready") is True
    except Exception:
        pass
    caps = (["asr", "diarization", "speech-translation"] if ok_t else []) + (["tts"] if ok_m else [])
    return {"capabilities": caps, "device": "auto", "ready": ok_t and ok_m,
            "transcribe": MODEL if ok_t else None, "tts": "magpietts" if ok_m else None}


@app.get("/health")
async def health():
    return await ready()


@app.get("/v1/models")
async def models():
    data = [{"capability": "transcription", "device": "auto", "id": "salm", "object": "model", "owned_by": "local",
             "speaker_diarization": True, "streaming": False, "word_timestamps": True, "root": MODEL},
            {"capability": "diarization", "device": "auto", "id": "diarization", "object": "model", "owned_by": "local"},
            {"capability": "speech-translation", "device": "auto", "id": "speech-translation", "object": "model",
             "owned_by": "local"}]
    try:
        for m in (await client.get(f"{MAGPIE}/v1/models", timeout=4)).json().get("data", []):
            if m.get("id") == "magpietts":
                data.append(m)
    except Exception:
        pass
    return {"object": "list", "data": data}


@app.post("/v1/chat/completions")
async def chat(request: Request):
    """Dictation path: rewrite input_audio parts to audio_url, force the official model + card defaults."""
    body = await request.json()
    for msg in body.get("messages", []):
        if isinstance(msg.get("content"), list):
            for part in msg["content"]:
                if part.get("type") == "input_audio":
                    ia = part.pop("input_audio")
                    part["type"] = "audio_url"
                    part["audio_url"] = {"url": data_url(base64.b64decode(ia["data"]), ia.get("format", "wav"))}
    body["model"] = MODEL
    body.setdefault("temperature", 0)
    ctk = body.setdefault("chat_template_kwargs", {})
    ctk.setdefault("enable_thinking", False)
    body.setdefault("skip_special_tokens", False)
    body.setdefault("spaces_between_special_tokens", False)
    stream = bool(body.get("stream"))
    if stream:
        r = await client.post(f"{VLLM}/v1/chat/completions", json=body)  # buffered; dictation clips are short
        return Response(r.content, status_code=r.status_code, media_type=r.headers.get("content-type", "text/event-stream"))
    r = await client.post(f"{VLLM}/v1/chat/completions", json=body)
    if r.status_code != 200:
        return Response(r.content, status_code=r.status_code, media_type="application/json")
    d = r.json()
    for ch in d.get("choices", []):
        m = ch.get("message") or {}
        if m.get("content"):
            m["content"] = clean(m["content"])
    d["model"] = "salm"
    d["transcription"] = d["choices"][0]["message"].get("content", "") if d.get("choices") else ""
    return d


def to_wav16k(raw: bytes) -> bytes:
    """Any audio -> 16 kHz mono s16 WAV (ffmpeg), so diarization turns can be sliced by sample."""
    import subprocess
    if raw[:4] == b"RIFF":
        import io, wave
        try:
            w = wave.open(io.BytesIO(raw))
            if w.getframerate() == 16000 and w.getnchannels() == 1 and w.getsampwidth() == 2:
                return raw
        except Exception:
            pass
    p = subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-i", "pipe:0", "-ac", "1", "-ar", "16000",
                        "-f", "wav", "pipe:1"], input=raw, capture_output=True, timeout=600)
    if p.returncode != 0:
        raise HTTPException(400, f"audio decode: {p.stderr[:200].decode(errors='replace')}")
    return p.stdout


def wav_slice(wav: bytes, start: float, end: float) -> bytes:
    import io, wave
    r = wave.open(io.BytesIO(wav)); sr = r.getframerate()
    a, b = max(0, int(start * sr)), min(r.getnframes(), int(end * sr))
    r.setpos(a); frames = r.readframes(max(0, b - a))
    out = io.BytesIO(); w = wave.open(out, "wb"); w.setnchannels(1); w.setsampwidth(2); w.setframerate(sr)
    w.writeframes(frames); w.close()
    return out.getvalue()


async def diarized_words(raw: bytes, wording: str = P_VERBATIM) -> tuple[list, float]:
    """Who-said-what that survives the model under-tagging speakers (2026-11-09 finding: the official CTC
    alignment forces untagged second-speaker words into speaker 0's turns). 1) diarization for the whole
    recording, 2) merge same-speaker turns (gap < 1 s), 3) transcribe each turn as single-speaker audio with
    word timestamps, in parallel, 4) shift word times back. Turn boundaries and speakers come from the
    audio-derived diarization, words from per-turn alignment."""
    import asyncio
    wav = to_wav16k(raw)
    dur = audio_seconds(wav) or 0.0
    a = ctc(await vllm_chat(P_SPEAKERS, data_url(wav), timestamps=True))
    segs = sorted(({"start": float(s["start"]), "end": float(s["end"]), "speaker": spk_int(s.get("speaker"))}
                   for s in a.get("diarization") or []), key=lambda s: s["start"])
    turns = []
    for s in segs:
        if turns and turns[-1]["speaker"] == s["speaker"] and s["start"] - turns[-1]["end"] < 1.0:
            turns[-1]["end"] = max(turns[-1]["end"], s["end"])
        else:
            turns.append(dict(s))
    if not turns:
        return [], dur

    async def one(t):
        pad0, pad1 = max(0.0, t["start"] - 0.15), min(dur or t["end"] + 0.15, t["end"] + 0.15)
        r = await vllm_chat(wording, data_url(wav_slice(wav, pad0, pad1)), timestamps=True)
        al = r.get("ctc_timestamps") or {}
        return [{"confidence": 0, "start": round(pad0 + float(w["start"]), 3), "end": round(pad0 + float(w["end"]), 3),
                 "speaker": t["speaker"], "word": w["word"]} for w in al.get("words", [])]

    parts = await asyncio.gather(*(one(t) for t in turns))
    return [w for p in parts for w in p], dur


@app.post("/v1/audio/transcriptions")
async def transcriptions(file: UploadFile = File(...), model: str = Form("salm"), language: str = Form(""),
                         prompt: str = Form(""), response_format: str = Form("json"),
                         diarization: str = Form("false"), timestamp_granularities: str = Form(""),
                         wording: str = Form("")):
    raw = await file.read()
    fmt = (file.filename or "a.wav").rsplit(".", 1)[-1]
    url = data_url(raw, fmt)
    diar = diarization.lower() in ("1", "true", "yes")
    wording_prompt = P_CLEAN if wording.lower() == "clean" else P_VERBATIM
    if diar:
        words, dur = await diarized_words(raw, wording_prompt)
        text = clean(" ".join(w["word"] for w in words))
        if response_format == "text":
            return PlainTextResponse(text)
        if response_format != "verbose_json":
            return {"text": text}
        return {"duration": dur, "language": language, "task": "transcribe", "text": text, "words": words}
    want_words = response_format == "verbose_json"
    p = P_SPEAKERS if diar else (P_CONTEXT.format(terms=prompt) if prompt else wording_prompt)
    resp = await vllm_chat(p, url, timestamps=want_words)
    text = clean(resp["choices"][0]["message"].get("content", ""))
    if response_format == "text":
        return PlainTextResponse(text)
    if response_format != "verbose_json":
        return {"text": text}
    a = ctc(resp)
    diar_segs = a.get("diarization") or []
    words = [{"confidence": 0, "end": round(float(w["end"]), 3),
              "speaker": speaker_at(diar_segs, float(w["start"]), float(w["end"])) or spk_int(w.get("speaker")),
              "start": round(float(w["start"]), 3), "word": w["word"]} for w in a.get("words", [])]
    dur = audio_seconds(raw) or (words[-1]["end"] if words else 0.0)
    return {"duration": dur, "language": language, "task": "transcribe", "text": text, "words": words}


@app.post("/v1/audio/diarizations")
async def diarizations(file: UploadFile = File(...)):
    raw = await file.read()
    resp = await vllm_chat(P_SPEAKERS, data_url(raw, (file.filename or "a.wav").rsplit(".", 1)[-1]), timestamps=True)
    a = ctc(resp)
    segs = [{"end": float(s["end"]), "speaker": spk_int(s.get("speaker")), "start": float(s["start"])}
            for s in a.get("diarization", [])]
    return {"segments": segs}


@app.post("/v1/audio/translations")
async def translations(file: UploadFile = File(...), language: str = Form("English"), response_format: str = Form("json")):
    raw = await file.read()
    resp = await vllm_chat(P_TRANSLATE.format(lang=language or "English"), data_url(raw, (file.filename or "a.wav").rsplit(".", 1)[-1]))
    text = clean(resp["choices"][0]["message"].get("content", ""))
    return PlainTextResponse(text) if response_format == "text" else {"text": text}


@app.post("/v1/audio/speech")
async def speech(request: Request):
    r = await client.post(f"{MAGPIE}/v1/audio/speech", content=await request.body(),
                          headers={"content-type": request.headers.get("content-type", "application/json")})
    return Response(r.content, status_code=r.status_code, media_type=r.headers.get("content-type", "audio/wav"))


@app.api_route("/{path:path}", methods=["GET", "POST"])
async def passthrough(path: str, request: Request):
    """Anything else goes to vLLM unchanged (tokenize, metrics, ...)."""
    r = await client.request(request.method, f"{VLLM}/{path}", content=await request.body(),
                             headers={"content-type": request.headers.get("content-type", "application/json")})
    return Response(r.content, status_code=r.status_code, media_type=r.headers.get("content-type"))
