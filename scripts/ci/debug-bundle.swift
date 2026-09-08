import AppKit
import CoreGraphics

// Inspect only the process launched by this smoke check. Window metadata proves
// native creation, not rendered WKWebView content or accessibility behavior.
guard CommandLine.arguments.count == 3,
      let pid = Int32(CommandLine.arguments[1]) else { exit(2) }
let expected = URL(fileURLWithPath: CommandLine.arguments[2]).standardizedFileURL
let deadline = Date().addingTimeInterval(25)
var readySince: Date?
while Date() < deadline {
    if let app = NSRunningApplication(processIdentifier: pid), !app.isTerminated,
       app.isFinishedLaunching, app.bundleIdentifier == "ai.xtrace.desktop",
       app.bundleURL?.standardizedFileURL == expected,
       let windows = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]],
       windows.contains(where: { item in
           guard (item[kCGWindowOwnerPID as String] as? Int32) == pid,
                 (item[kCGWindowLayer as String] as? Int) == 0,
                 let bounds = item[kCGWindowBounds as String] as? [String: Any],
                 let width = bounds["Width"] as? Double,
                 let height = bounds["Height"] as? Double else { return false }
           return width >= 1100 && height >= 700
       }) {
        if let readySince, Date().timeIntervalSince(readySince) >= 2 {
            print("Native bundle finished launching and created its main window.")
            exit(0)
        }
        if readySince == nil { readySince = Date() }
    } else {
        readySince = nil
    }
    RunLoop.current.run(until: Date().addingTimeInterval(0.2))
}
fputs("Native bundle did not reach the required launch/window state.\n", stderr)
exit(1)
