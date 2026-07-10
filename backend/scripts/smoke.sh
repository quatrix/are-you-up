#!/usr/bin/env bash
# E2E smoke: start the real server, POST a synthetic evening, assert the
# derived intervals byte-for-byte. Exits non-zero on any mismatch.
set -euo pipefail
cd "$(dirname "$0")/.."

PORT=$(( (RANDOM % 20000) + 20000 ))
DB="$(mktemp -d)/smoke.db"
BASE="http://127.0.0.1:$PORT"

cargo build --quiet
ARE_YOU_UP_ADDR="127.0.0.1:$PORT" ARE_YOU_UP_DB="$DB" ./target/debug/are-you-up-backend &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
    if curl -sf "$BASE/healthz" >/dev/null 2>&1; then break; fi
    sleep 0.1
done

curl -sf -X POST "$BASE/v1/samples" -H 'content-type: application/json' -d '{
  "source": "smoke",
  "samples": [
    {"ts": "2026-07-10T22:00:00+03:00", "idle_s": 5},
    {"ts": "2026-07-10T22:00:30+03:00", "idle_s": 2},
    {"ts": "2026-07-10T22:01:00+03:00", "idle_s": 9},
    {"ts": "2026-07-10T22:01:30+03:00", "idle_s": 1000},
    {"ts": "2026-07-10T22:02:00+03:00", "idle_s": 1030},
    {"ts": "2026-07-10T22:10:00+03:00", "idle_s": 3},
    {"ts": "2026-07-10T22:10:30+03:00", "idle_s": 4}
  ]}' >/dev/null

RESULT="$(curl -sf "$BASE/v1/intervals?from=2026-07-10T22:00:00%2B03:00&to=2026-07-10T23:00:00%2B03:00&source=smoke")"

python3 - "$RESULT" <<'EOF'
import json, sys
intervals = json.loads(sys.argv[1])["intervals"]
expected = [
    ("active", "2026-07-10T22:00:00+03:00", "2026-07-10T22:01:00+03:00"),
    ("idle",   "2026-07-10T22:01:30+03:00", "2026-07-10T22:02:00+03:00"),
    ("active", "2026-07-10T22:10:00+03:00", "2026-07-10T22:10:30+03:00"),
]
actual = [(i["state"], i["start"], i["end"]) for i in intervals]
assert actual == expected, f"unexpected intervals: {actual}"
print("smoke OK")
EOF
