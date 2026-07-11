# are-you-up backend

REST server that stores raw activity samples and serves derived
active/idle intervals. See `../docs/superpowers/specs/` for the design.

## Run

    make run                  # or: cargo run
    ARE_YOU_UP_ADDR=127.0.0.1:8080 ARE_YOU_UP_DB=./are-you-up.db make run

Both env vars are optional; the values above are the defaults. Deploy by
binding the tailnet address. `make build` produces a release binary.

## API

    POST /v1/samples
      {"source": "macbook", "samples": [{"ts": "2026-07-10T23:41:03+03:00", "idle_s": 4}]}
      -> {"accepted": 1}       (upsert on (source, ts); retries are harmless)

    GET /v1/intervals?from=...&to=...&threshold_s=900&source=macbook
      -> {"intervals": [{"source", "start", "end", "state": "active"|"idle"}]}
      from/to are RFC 3339 and required. Percent-encode "+" offsets (%2B).
      threshold_s (default 900): seconds without input before time counts
      as idle. Gaps in samples > 90s are returned as no interval at all.

    GET /healthz -> "ok"

## Test

    make test             # unit + integration (cargo test)
    make smoke            # E2E against a real server process
    ../scripts/e2e.sh     # joint E2E: real mac client syncing to this server
