# Why digits said several times in a row went missing

Found 2026-11-10 while checking the voices of Lumbridge Speak: a phone number spoken as
"three one five, five five five, zero one three seven" came back from the speech front
(`:8806`, Nemotron 3.5 Transcribe) as "three one five five five zero one three seven".
One of the three fives was gone, in every voice.

## Cause

The instruction the front sends with a plain `POST /v1/audio/transcriptions`. It was the
model card's "clean written transcript" prompt:

> Audio transcription. Language: as spoken. Wording: **omit fillers and accidental
> repetitions**; keep the remaining spoken wording unchanged. ...

The model does what it is told: a digit said two or three times in a row is treated as
a repetition and thinned out. The same prompt was used for every turn of a diarized
transcription.

It is not the decoding. Every request below ran with the same settings (temperature 0,
seed 0, no repetition penalty, the same speculative decoding), and only the instruction
changed. The front does no post-processing that could drop words (`clean()` only removes
tags and extra spaces), and the loop guard from the model card is not installed.

## Measurements

Clips made with Magpie (voices Leo, Jason, Aria, John, Sofia), five sentences: two phone
numbers, a six-digit code, "port eight eight eight eight", and a PIN with a triple and a
double. A clip counts when every digit is in the transcript. A second, smaller literal
transcription model hears every digit in these clips, so the audio does contain them.
All requests went to the running front as ordinary requests; nothing on the server was
changed.

| Instruction | Clips with every digit |
|---|---|
| Clean (the front's default until now) | 4 of 25 |
| Verbatim (model card) | 25 of 25 |
| Clean, plus a sentence asking to keep repeated digits (three wordings) | 6, 7 and 4 of 25 |

With three voices (15 clips): clean 2, verbatim 14, the card's "numbers / spelled
letters" prompt 14, context terms (what a `prompt` field selects) 14, the Mac app's
dictation instruction 14, and the dictation instruction with an added sentence about
repeats or written numbers 14. The one miss in each was the same clip.

So the wording clause decides it, and adding an exception to the clean prompt does not
help: the model follows the prompt families it was trained on.

The clean prompt also removes repetition that was meant: "No, no, no, that is really,
really good" came back as "No, that is really good."

## Who is affected

- **Affected:** every caller of `/v1/audio/transcriptions` that sends no `prompt`, with or
  without diarization, and the Mac app only when its dictation request fails and it falls
  back to the plain route.
- **Not affected:** dictation from the Mac app. It uses `/v1/chat/completions` with its
  own instruction ("Transcribe this dictation verbatim ..."), which kept the digits in
  14 of 15 clips. If digits go missing in a dictation, the cause is something else; a
  recording of such a dictation from the app's history would be the thing to test.

These clips are synthetic speech. A person runs digits together more than Magpie does,
so real dictation should be checked with a few recordings before calling it settled.

## Fix

In this repository, both fronts (`front-py/speech_front.py`, `src/`): plain and diarized
transcription use the verbatim instruction by default. The clean one stays available as
the form field `wording=clean`. The price is that fillers ("um", "uh") stay in the
transcript by default.

With a front older than this change, a caller can avoid the loss today by sending a
`prompt` (context terms): that selects another instruction, which keeps the digits.

