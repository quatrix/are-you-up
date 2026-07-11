#!/usr/bin/env bash
# Joint E2E (mac plan Task 8): real backend + real mac client, no stubs.
# The client samples every 30s and syncs every 60s; we run it ~75s so at
# least one sync cycle completes, then ask the server for intervals.
set -euo pipefail
cd "$(dirname "$0")/.."

PORT=18080
E2E_HOME="$PWD/tmp/e2e-home"
DB="$(mktemp -d)/e2e.db"
BASE="http://127.0.0.1:$PORT"

rm -rf "$E2E_HOME"
mkdir -p "$E2E_HOME/Library/Application Support/are-you-up"
printf '{"server_url": "http://127.0.0.1:%s", "source": "e2e-mac"}' "$PORT" \
    > "$E2E_HOME/Library/Application Support/are-you-up/config.json"

(cd backend && cargo build --quiet)
ARE_YOU_UP_ADDR="127.0.0.1:$PORT" ARE_YOU_UP_DB="$DB" ./backend/target/debug/are-you-up-backend &
SERVER_PID=$!
(cd mac && swift build --quiet)
ARE_YOU_UP_HOME="$E2E_HOME" ./mac/.build/debug/are-you-up &
APP_PID=$!
trap 'kill "$SERVER_PID" "$APP_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
    if curl -sf "$BASE/healthz" >/dev/null 2>&1; then break; fi
    sleep 0.1
done

sleep 75

FROM="$(python3 -c "from datetime import datetime, timedelta, timezone; from urllib.parse import quote; print(quote((datetime.now(timezone.utc).astimezone() - timedelta(minutes=10)).isoformat(timespec='seconds')))")"
TO="$(python3 -c "from datetime import datetime, timedelta, timezone; from urllib.parse import quote; print(quote((datetime.now(timezone.utc).astimezone() + timedelta(minutes=10)).isoformat(timespec='seconds')))")"
RESULT="$(curl -sf "$BASE/v1/intervals?from=$FROM&to=$TO&source=e2e-mac")"

echo "--- server response ---"
echo "$RESULT"
echo "--- client log ---"
cat "$E2E_HOME/Library/Logs/are-you-up.log"

python3 - "$RESULT" <<'EOF'
import json, sys
intervals = json.loads(sys.argv[1])["intervals"]
assert len(intervals) >= 1, f"expected at least one interval, got: {intervals}"
assert all(i["source"] == "e2e-mac" for i in intervals)
print("joint E2E OK")
EOF
