# Security policy

## Reporting a problem

Please do not open a public issue, pull request or discussion for a security problem.

Email **security@lumbridgecorp.com**, or use "Report a vulnerability" on this repo's Security tab.

Please include:

- The version or commit (`git rev-parse HEAD`), and whether it is the front or the Mac app.
- Your operating system and CPU type.
- What you did, what happened, and what you expected.
- Whether it needs local access, network access, or another account.
- The impact you think it has.
- A small way to reproduce it, if you have one.

Remove passwords, tokens, recordings and private data from logs before you send them.

## What happens next

- We reply to say we have your report.
- We tell you whether we can reproduce it and what we plan to do.
- We keep you informed until it is fixed or closed.
- We credit you in the change log when the fix ships, unless you ask us not to.

Please give us a fair chance to fix the problem before you publish it. We ask for 90 days, or less if
a fix is out sooner.

## Supported versions

The latest release and the latest commit on `main` are supported. There are no back-ports.

## How Lumbridge Voice is meant to be used

- **The front has no authentication.** Anyone who can reach its port can transcribe audio on your model
  server and reach the model server's other routes through it (unknown paths are handed on unchanged).
  It listens on loopback by default. Keep it there, or behind a private network or a reverse proxy that
  authenticates. Do not expose it to the internet.
- The front stores nothing: no audio, no transcripts. Requests are limited in size (`max_upload_mb`).
- It runs `ffmpeg` on uploaded audio for who-said-what. Keep `ffmpeg` up to date.
- The Mac app runs as you, with Microphone, Accessibility and Input Monitoring permissions: it reads the
  right Option key, types text into the frontmost app and reads the selected text when you ask it to.
- The Mac app's announce server (port 8796) accepts loopback and mesh VPN addresses only and needs a
  random token kept in its support folder (mode 0600). Anyone with that token can make the Mac speak.
- Ask sends a screenshot to the chat model you set. Text in the screenshot is passed as data, but a model
  can still be misled by it.

## What counts

In scope:

- A request to the front that reads or writes files, runs a program other than `ffmpeg` on the upload,
  or reaches an address other than the configured model and speech servers.
- A way to reach the Mac app's announce server or its commands without the token, or from another network.
- Leaks of tokens, audio or transcripts to logs, other users or the network.
- A secret or private data committed to this repository.

Out of scope:

- The front's missing authentication, as described above.
- Anything that needs an attacker who already has your user account on the machine.
- The model servers you run behind the front. Report those to their owners.
- Denial of service by sending large or many requests to a front you exposed.

## Good faith

We will not take legal action against anyone who finds and reports a problem in good faith, keeps to
this policy, and does not access or change other people's data.
