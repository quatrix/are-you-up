import AppKit

// Accessory: menu bar only, no dock icon, no main window.
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = AppDelegate()
app.delegate = delegate
app.run()
