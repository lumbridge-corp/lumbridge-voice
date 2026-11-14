# Contributing

Report bugs and request features as issues in this repository. Discuss bigger changes there before writing a
PR. Report security bugs by email instead: see [SECURITY.md](SECURITY.md). Keep PRs small and add tests for
behavior changes.

## Setup

Install Rust (1.85 or newer; CI uses 1.97.1) with `rustfmt` and `clippy`, Python 3, and `ffmpeg` for the
tests that need it. The Mac app needs macOS and the Xcode Command Line Tools.

## Checks

```sh
scripts/ci.sh                      # format, clippy, tests, syntax checks: what CI runs
cargo test -- --include-ignored    # also the test that needs ffmpeg
```

The tests run the API against a mock model. `tests/fixtures/observed/` holds real answers of the Python
front, and the Rust front must answer with the same bytes. `tests/fixtures/cases.json` holds the Python
front's answers for its helper functions; `python3 tests/fixtures/gen_cases.py` regenerates it from
`front-py/`. One test, off by default, asks a real model server (see the end of `tests/api.rs`).

A change to the API changes `docs/API.md` in the same PR. A new model is a new adapter (`src/adapter.rs`),
not a change to the routes.

For the Mac app, build with `mac/build.sh` and try it with `mac/install.sh` on your own Mac.

Before a PR, run `scripts/scrub-check`. It fails on private network addresses, home folders, email
addresses and token formats that are not listed in `.scrub-allow`. Use made-up examples: user `sam`, host
`desk`, `example.com`, `192.0.2.x`. Never commit audio, voiceprints, model weights or tokens.

PRs target `main`. Write what users will notice in CHANGELOG.md. By sending a PR you agree that your change
is licensed under Apache-2.0, like the rest of the project.
