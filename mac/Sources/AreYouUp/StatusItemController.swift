import AppKit
import AreYouUpCore

/// Menu bar presence: icon reflecting state, menu with status, last sync,
/// 6h history strip, pause/resume, quit.
final class StatusItemController: NSObject {
    private let item: NSStatusItem
    private let stateLine = NSMenuItem(title: "starting...", action: nil, keyEquivalent: "")
    private let syncLine = NSMenuItem(title: "Last sync: never", action: nil, keyEquivalent: "")
    private let pauseItem: NSMenuItem
    private let historyView = HistoryStripView(frame: NSRect(x: 0, y: 0, width: 280, height: 40))
    private let onTogglePause: () -> Void
    private let onQuit: () -> Void

    init(onTogglePause: @escaping () -> Void, onQuit: @escaping () -> Void) {
        self.onTogglePause = onTogglePause
        self.onQuit = onQuit
        self.item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        self.pauseItem = NSMenuItem(title: "Pause",
                                    action: #selector(StatusItemController.togglePause),
                                    keyEquivalent: "")
        super.init()

        item.button?.title = "●"
        let menu = NSMenu()
        menu.autoenablesItems = false
        stateLine.isEnabled = false
        syncLine.isEnabled = false
        menu.addItem(stateLine)
        menu.addItem(syncLine)
        menu.addItem(.separator())
        let historyItem = NSMenuItem()
        historyItem.view = historyView
        menu.addItem(historyItem)
        menu.addItem(.separator())
        pauseItem.target = self
        menu.addItem(pauseItem)
        let quitItem = NSMenuItem(title: "Quit", action: #selector(StatusItemController.quit),
                                  keyEquivalent: "q")
        quitItem.target = self
        menu.addItem(quitItem)
        item.menu = menu
    }

    func update(idleS: Int, paused: Bool, lastSuccess: Date?, history: [Sample]) {
        let active = idleS < AppDelegate.displayThresholdS
        item.button?.title = paused ? "⏸" : (active ? "●" : "○")
        stateLine.title = paused ? "Paused" : (active ? "Active (idle \(idleS)s)" : "Idle (\(idleS)s)")
        syncLine.title = "Last sync: " + (lastSuccess.map(relative) ?? "never")
        pauseItem.title = paused ? "Resume" : "Pause"
        historyView.samples = history
    }

    private func relative(_ date: Date) -> String {
        let s = Int(-date.timeIntervalSinceNow)
        if s < 60 { return "\(s)s ago" }
        if s < 3600 { return "\(s / 60)m ago" }
        return "\(s / 3600)h \(s % 3600 / 60)m ago"
    }

    @objc private func togglePause() { onTogglePause() }
    @objc private func quit() { onQuit() }
}
