#!/usr/bin/env python3
"""Writes cases.json: what the Python front's own helper functions answer for a set of inputs.

The Rust tests compare against this file, so the two fronts clean text, number speakers,
read WAV files and cut audio the same way.
Run from the repo root:  python3 tests/fixtures/gen_cases.py

The front itself needs FastAPI to import, so this picks the plain helper functions out
of its source and runs only those. Nothing is started.
"""
import ast, base64, io, json, os, struct, wave

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(HERE, "..", "..", "front-py", "speech_front.py")
WANT = {"data_url", "clean", "spk_int", "speaker_at", "audio_seconds", "wav_slice", "TAG", "MIME"}

tree = ast.parse(open(SRC).read())
keep = [n for n in tree.body
        if (isinstance(n, ast.FunctionDef) and n.name in WANT)
        or (isinstance(n, ast.Assign) and getattr(n.targets[0], "id", None) in WANT)]
assert {getattr(n, "name", None) or n.targets[0].id for n in keep} == WANT
ns = {}
exec("import base64, re\n" + "\n".join(ast.unparse(n) for n in keep), ns)


def tone(rate, seconds, channels=1, width=2):
    """A WAV with a counting pattern, so a slice can be recognised."""
    n = int(rate * seconds)
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(channels); w.setsampwidth(width); w.setframerate(rate)
        w.writeframes(b"".join(struct.pack("<h", (i * 7) % 30000 - 15000) for i in range(n * channels)) if width == 2
                      else bytes((i * 7) % 256 for i in range(n * channels * width)))
    return buf.getvalue()


def b64(b):
    return base64.b64encode(b).decode()


out = {}

out["clean"] = [{"in": s, "out": ns["clean"](s)} for s in [
    "<spk:0> Testing one to three.",
    "<spk:0> Hello there. <spk:1> Hi!  <spk:12>   Bye.",
    "<|startoftranscript|><|en|> Plain   text <|endoftext|>",
    "<s> wrapped </s>",
    "  \n tabs\tand\nnewlines \r\n ",
    "keeps <b>other</b> tags, <spk:x> and < spk:1 >",
    "<|a|b|> stays, <||> goes, <|ok|> goes",
    "café 日本語 <spk:3> naïve",
    "",
    "<spk:0>",
]]
out["clean"].append({"in": None, "out": ns["clean"](None)})

out["spk_int"] = [{"in": v, "out": ns["spk_int"](v)} for v in
                  ["<spk:0>", "<spk:3>", "spk_0", "spk_11", "0", "2", 0, 1, 7, None, "", "speaker", 1.0, "<spk:2> and 9"]]

segs = [{"start": 0.0, "end": 2.0, "speaker": 0}, {"start": 2.0, "end": 5.0, "speaker": "1"},
        {"start": 8.0, "end": 9.0, "speaker": "<spk:2>"}, {"start": 9.5, "end": 9.6}]
out["speaker_at"] = [{"segs": s, "start": a, "end": b, "out": ns["speaker_at"](s, a, b)} for s, a, b in [
    (segs, 0.1, 0.5), (segs, 1.8, 2.3), (segs, 1.9, 2.05), (segs, 2.0, 2.0), (segs, 5.5, 6.0), (segs, 7.2, 7.9),
    (segs, 6.4, 6.6), (segs, 9.55, 9.58), (segs, 20.0, 21.0), (segs, 4.9, 8.2), ([], 1.0, 2.0),
    ([{"start": 1.0, "end": 2.0, "speaker": 4}], 0.0, 0.5),
]]

mono16 = tone(16000, 0.25)
piped = bytearray(mono16)  # what ffmpeg writes to a pipe: lengths it could not go back and fill in
piped[4:8] = b"\xff\xff\xff\xff"; piped[40:44] = b"\xff\xff\xff\xff"
with_list = mono16[:36] + b"LIST" + struct.pack("<I", 5) + b"INFO!" + b"\0" + mono16[36:]  # odd-sized chunk before data
floaty = bytearray(tone(16000, 0.05)); floaty[20:22] = struct.pack("<H", 3)  # IEEE float format tag
wavs = {
    "mono16k": mono16,
    "mono22k": tone(22050, 0.1),
    "stereo44k": tone(44100, 0.05, channels=2),
    "mono8bit": tone(8000, 0.1, width=1),
    "empty": tone(16000, 0),
    "with_list_chunk": with_list,
    "float_format": bytes(floaty),
    "truncated_header": mono16[:30],
    "not_riff": b"OggS" + mono16[4:200],
    "riff_not_wave": b"RIFF" + mono16[4:8] + b"AVI " + mono16[12:200],
    "piped_lengths": bytes(piped),
}
out["wavs"] = {k: b64(v) for k, v in wavs.items()}
out["audio_seconds"] = [{"wav": k, "out": ns["audio_seconds"](v)} for k, v in wavs.items()]
# What Python says for "piped_lengths" is the fault described in docs/API.md (134217.7 s).
# The Rust front reads the real length instead; its test names this case.

out["wav_slice"] = []
for name, a, b in [("mono16k", 0.0, 0.25), ("mono16k", 0.05, 0.1), ("mono16k", 0.0123, 0.14567),
                   ("mono16k", 0.2, 5.0), ("mono16k", 0.1, 0.1), ("mono16k", 0.2, 0.1),
                   ("mono16k", -1.0, 0.02), ("with_list_chunk", 0.1, 0.2), ("piped_lengths", 0.2, 0.4),
                   ("stereo44k", 0.01, 0.02)]:
    out["wav_slice"].append({"wav": name, "start": a, "end": b, "out": b64(ns["wav_slice"](wavs[name], a, b))})

out["data_url"] = [{"fmt": f, "out": ns["data_url"](b"abc", f)} for f in
                   ["wav", "WAV", "mp3", "flac", "ogg", "webm", "m4a", "mp4", "opus", "", "a"]]

path = os.path.join(HERE, "cases.json")
with open(path, "w") as f:
    json.dump(out, f, indent=1, ensure_ascii=True)
    f.write("\n")
print({k: len(v) for k, v in out.items()}, "->", path, os.path.getsize(path), "bytes")
