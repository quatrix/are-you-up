import AppKit
import AreYouUpCore

final class AppDelegate: NSObject, NSApplicationDelegate {
    static let samplePeriod: TimeInterval = 30
    static let syncPeriod: TimeInterval = 60
    /// Display-only: the server owns real classification (ADR-0002). Matches
    /// the server's default threshold_s.
    static let displayThresholdS = 900

    private var log: Log!
    private var store: Store!
    private var syncer: Syncer!
    private var statusItem: StatusItemController!
    private var paused = false
    // Syncer.syncOnce is not re-entrant: a drain slower than the sync timer
    // (big backlog over a slow link) must not overlap the next tick.
    // Double-send would be idempotent server-side, but wasteful.
    private var syncInFlight = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        // ARE_YOU_UP_HOME redirects all state for tests/E2E runs.
        let home = ProcessInfo.processInfo.environment["ARE_YOU_UP_HOME"] ?? NSHomeDirectory()
        let appSupport = home + "/Library/Application Support/are-you-up"
        let configPath = appSupport + "/config.json"
        let dbPath = appSupport + "/client.db"
        log = Log(path: home + "/Library/Logs/are-you-up.log",
                  debugEnabled: ProcessInfo.processInfo.environment["ARE_YOU_UP_DEBUG"] == "1")
        do {
            let config = try Config.load(path: configPath)
            guard let serverURL = URL(string: config.serverURL) else {
                log.error("config server_url is not a URL: \(config.serverURL) (edit \(configPath))")
                NSApp.terminate(nil)
                return
            }
            store = try Store(path: dbPath)
            syncer = Syncer(store: store, serverURL: serverURL, source: config.source)
        } catch {
            // DecodingError/StoreError carry no path of their own; name the
            // files so the user can find and fix the typo the error is about.
            log.error("startup failed (config: \(configPath), db: \(dbPath)): \(error)")
            NSApp.terminate(nil)
            return
        }

        statusItem = StatusItemController(
            onTogglePause: { [weak self] in self?.togglePause() },
            onQuit: { NSApp.terminate(nil) }
        )
        log.info("started")

        Timer.scheduledTimer(withTimeInterval: Self.samplePeriod, repeats: true) { [weak self] _ in
            self?.sampleTick()
        }
        Timer.scheduledTimer(withTimeInterval: Self.syncPeriod, repeats: true) { [weak self] _ in
            self?.syncTick()
        }
        sampleTick()
        syncTick()
    }

    private func sampleTick() {
        guard !paused else { return }
        let idleS = IdleTime.secondsSinceLastInput()
        do {
            try store.insert(Sample(ts: Timestamps.now(), idleS: idleS))
            try store.pruneSynced(before: Timestamps.string(from: Date().addingTimeInterval(-7 * 86_400)))
        } catch {
            log.error("storing sample: \(error)")
        }
        log.debug("sample idle_s=\(idleS)")
        refreshStatus(idleS: idleS)
    }

    private func syncTick() {
        guard !paused, !syncInFlight else { return }
        syncInFlight = true
        syncer.syncOnce { [weak self] synced, failure in
            guard let self else { return }
            self.syncInFlight = false
            if let failure {
                self.log.info("sync failed after \(synced) samples, will retry: \(failure)")
            } else if synced > 0 {
                self.log.info("synced \(synced) samples")
            } else {
                self.log.debug("sync ok, nothing new")
            }
            self.refreshStatus(idleS: IdleTime.secondsSinceLastInput())
        }
    }

    private func togglePause() {
        paused.toggle()
        log.info(paused ? "paused" : "resumed")
        refreshStatus(idleS: IdleTime.secondsSinceLastInput())
    }

    private func refreshStatus(idleS: Int) {
        let since = Timestamps.string(from: Date().addingTimeInterval(-6 * 3600))
        let history = (try? store.samples(since: since)) ?? []
        statusItem.update(idleS: idleS, paused: paused,
                          lastSuccess: syncer.lastSuccess, history: history)
    }
}
