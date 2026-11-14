#!/usr/bin/env bash
# What CI runs, and what to run before a pull request: scripts/ci.sh
# The Rust front: format, clippy, tests (against the mock model; no model server, no network).
# The rest: syntax checks of the Python and shell files. The Mac app needs macOS to build (mac/build.sh).
set -euo pipefail
cd "$(dirname "$0")/.."
for v in $(env | grep -oE '^LUMBRIDGE_VOICE_[A-Za-z0-9_]*' || true); do unset "$v"; done
rustc --version
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
T=$(mktemp -d "${TMPDIR:-/tmp}/lvci.XXXXXX")
trap 'rm -rf "$T"' EXIT
for f in mac/lumbridge-voice front-py/speech_front.py tests/fixtures/gen_cases.py; do
  python3 -c 'import ast, sys; ast.parse(open(sys.argv[1]).read(), sys.argv[1])' "$f"
done
for f in mac/build.sh mac/install.sh examples/serve-transcribe-vllm.sh scripts/ci.sh; do bash -n "$f"; done
python3 -m json.tool vocab/boosting.example.json >/dev/null
echo "ci: ok"
