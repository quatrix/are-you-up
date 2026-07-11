# are-you-up mac client

Menu bar app that records seconds-since-last-input every 30s into local
sqlite and syncs to the backend every 60s. See
`../docs/superpowers/specs/` for the design.

## Paths

- Config:  `~/Library/Application Support/are-you-up/config.json`
  (`{"server_url": "...", "source": "macbook"}`, created on first run)
- Data:    `~/Library/Application Support/are-you-up/client.db`
- Log:     `~/Library/Logs/are-you-up.log` (tail it; set
  `ARE_YOU_UP_DEBUG=1` for per-sample lines)
- `ARE_YOU_UP_HOME` env var redirects all of the above (used by tests).

## Run / install

    make run                  # foreground run (swift run are-you-up)
    make install              # release build + LaunchAgent (starts at login)
    make uninstall

## Menu

Icon: ● active, ○ idle (no input for 15 min), ⏸ paused. The menu shows
current status, last successful sync, a 6-hour history strip (green
active / gray idle / empty no-data), Pause/Resume, and Quit. Pausing
stops sampling entirely; paused time uploads nothing.

## Test

    make test
