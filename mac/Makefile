PREFIX ?= $(HOME)/.local
BIN = $(PREFIX)/bin/are-you-up
PLIST = $(HOME)/Library/LaunchAgents/com.are-you-up.mac.plist

.PHONY: build test install uninstall

build:
	swift build -c release

test:
	swift test

install: build
	mkdir -p $(PREFIX)/bin
	cp .build/release/are-you-up $(BIN)
	mkdir -p $(HOME)/Library/LaunchAgents
	sed "s|@BIN@|$(BIN)|" launchd/com.are-you-up.mac.plist > $(PLIST)
	launchctl bootout gui/$$(id -u) $(PLIST) 2>/dev/null || true
	launchctl bootstrap gui/$$(id -u) $(PLIST)

uninstall:
	launchctl bootout gui/$$(id -u) $(PLIST) 2>/dev/null || true
	rm -f $(PLIST) $(BIN)
