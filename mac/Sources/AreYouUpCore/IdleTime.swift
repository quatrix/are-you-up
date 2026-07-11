import CoreGraphics
import Foundation

public enum IdleTime {
    /// Raw stopwatch: seconds since the last input event of ANY type
    /// (kCGAnyInputEventType). Needs no TCC/accessibility permission
    /// (verified on the target machine, LAB_NOTES.md 2026-07-10).
    ///
    /// This counts AWAKE time, not wall clock - it pauses during sleep
    /// (LAB_NOTES 2026-07-11). Never report it directly; feed it through
    /// `WallClockIdle`.
    public static func rawSecondsSinceLastInput() -> TimeInterval {
        // kCGAnyInputEventType is ~0; the Swift enum initializer accepts it.
        // Fall back to mouseMoved if an SDK ever rejects the raw value.
        let anyInput = CGEventType(rawValue: ~UInt32(0)) ?? .mouseMoved
        let seconds = CGEventSource.secondsSinceLastEventType(.combinedSessionState,
                                                              eventType: anyInput)
        return max(0, seconds)
    }
}
