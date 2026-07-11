# are-you-up backend

REST server that stores raw activity samples and serves derived
active/idle intervals. See `../docs/superpowers/specs/` for the design.

## Run

    make run                                          # defaults: 127.0.0.1:8080, ./are-you-up.db
    cargo run -- --addr 0.0.0.0:8080 --db /var/lib/are-you-up.db
    cargo run -- --help                               # full options

Every option resolves as: flag, then env var (`ARE_YOU_UP_ADDR`,
`ARE_YOU_UP_DB`), then default - existing env-based setups keep working.
Deploy by binding the tailnet address. `make build` produces a release
binary.

Logging goes to stdout via `tracing`; `RUST_LOG` controls verbosity
(default `info`):

    RUST_LOG=debug make run       # per-request traces, handler events, 4xx reasons
    RUST_LOG=are_you_up_backend=debug,tower_http=info make run

Server faults (5xx) always log at `error`; client mistakes (4xx) only
appear at `debug`.

## Run as a systemd service (Ubuntu)

One-time setup, on the server:

    # prerequisites: a C compiler (bundled sqlite) and rust via rustup
    # (rustup honors backend/rust-toolchain.toml automatically)
    sudo apt install -y build-essential curl
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

    git clone https://github.com/quatrix/are-you-up.git
    cd are-you-up/backend
    cargo build --release
    sudo cp target/release/are-you-up-backend /usr/local/bin/
    sudo cp systemd/are-you-up.service /etc/systemd/system/
    sudo systemctl daemon-reload
    sudo systemctl enable --now are-you-up

The unit (`systemd/are-you-up.service`) runs as a transient
unprivileged user (`DynamicUser`) with the database in
`/var/lib/are-you-up/` (`StateDirectory`, created and owned by
systemd), restarts on failure, and logs to the journal.

Operating it:

    systemctl status are-you-up
    journalctl -u are-you-up -f              # follow the logs
    sudo systemctl edit are-you-up           # drop-in override, e.g. Environment=RUST_LOG=debug
    sudo systemctl restart are-you-up

The shipped unit binds `0.0.0.0:8080`. The API has no auth by design,
so if the box has interfaces outside your tailnet, edit `--addr` in
`ExecStart` to the machine's tailnet IP (and uncomment the
`After=tailscaled.service` line so the bind happens after tailscale is
up).

Upgrading:

    cd are-you-up && git pull
    cd backend && cargo build --release
    sudo cp target/release/are-you-up-backend /usr/local/bin/
    sudo systemctl restart are-you-up

## API

    POST /v1/samples
      {"source": "macbook", "samples": [{"ts": "2026-07-10T23:41:03+03:00", "idle_s": 4}]}
      -> {"accepted": 1}       (upsert on (source, ts); retries are harmless)

    GET /v1/intervals?from=...&to=...&threshold_s=900&source=macbook
      -> {"intervals": [{"source", "start", "end", "state": "active"|"idle"}]}
      from/to are RFC 3339 and required. Percent-encode "+" offsets (%2B).
      threshold_s (default 900): seconds without input before time counts
      as idle. Gaps in samples > 90s are returned as no interval at all.

    GET /v1/intervals?from=...&to=...&consolidate=true
      -> {"intervals": [{"start", "end", "sources": ["macbook", "pixel"]}]}
      The cross-source awake-evidence view: only ACTIVE time, unioned
      across sources and split wherever the set of active sources changes
      (so "sources" is exact per interval; sorted). No "state" field -
      every interval is active by definition; idle is not evidence and is
      not returned. consolidate must be exactly true or false (else 400);
      composes with threshold_s and source.

    GET / -> the timeline visualization (self-contained HTML page):
      date pickers + today/last-week/last-month presets, one 24h line
      per day showing consolidated awake time (macbook / pixel / both).

    GET /healthz -> "ok"

## Test

    make test             # unit + integration (cargo test)
    make smoke            # E2E against a real server process
    ../scripts/e2e.sh     # joint E2E: real mac client syncing to this server
