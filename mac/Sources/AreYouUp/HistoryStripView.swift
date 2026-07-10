import AppKit
import AreYouUpCore

/// Draws the last 6 hours as colored segments: green = active, gray = idle,
/// background showing through = no data (asleep, off, or paused).
final class HistoryStripView: NSView {
    var samples: [Sample] = [] {
        didSet { needsDisplay = true }
    }

    private let window6h: TimeInterval = 6 * 3600
    private let samplePeriod: TimeInterval = AppDelegate.samplePeriod

    override func draw(_ dirtyRect: NSRect) {
        let inset = bounds.insetBy(dx: 12, dy: 12)
        NSColor.quaternaryLabelColor.setFill()
        NSBezierPath(roundedRect: inset, xRadius: 3, yRadius: 3).fill()

        let end = Date()
        let start = end.addingTimeInterval(-window6h)
        for sample in samples {
            guard let t = Timestamps.date(from: sample.ts), t >= start, t <= end else { continue }
            let x = inset.minX + CGFloat(t.timeIntervalSince(start) / window6h) * inset.width
            let width = max(1, CGFloat(samplePeriod / window6h) * inset.width)
            let active = sample.idleS < AppDelegate.displayThresholdS
            (active ? NSColor.systemGreen : NSColor.systemGray).setFill()
            NSRect(x: x, y: inset.minY, width: width, height: inset.height).fill()
        }
    }
}
