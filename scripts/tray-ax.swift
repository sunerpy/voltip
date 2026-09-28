// Drive another process's menu bar status item through the Accessibility API, for
// scripts/smoke-tray-macos.sh. Needs the Accessibility permission (the smoke grants it to the
// compiled binary in the runner's TCC database).
//
//   tray-ax <pid> frame              x y width height of the status item (points)
//   tray-ax <pid> menu               open the item's menu, print its entries, close it
//   tray-ax <pid> press <title>      open the menu and choose the entry titled <title>
//   tray-ax <pid> windows            the titles of the process's windows, one per line
//   tray-ax <pid> close <title>      press the close button of the window titled <title>
//   tray-ax <pid> dialog <name>      exit 0 when a window holds a dialog named <name>
//                                    (a webview's role=dialog: AXApplicationDialog)
//   tray-ax ink <png>                "ink=<share> blue=<share>": the pixels that stand out from
//                                    the image's median brightness, and the saturated blue ones
import ApplicationServices
import Foundation
import ImageIO

func fail(_ message: String) -> Never {
    FileHandle.standardError.write("tray-ax: \(message)\n".data(using: .utf8)!)
    exit(1)
}

func value(_ element: AXUIElement, _ name: String) -> AnyObject? {
    var out: AnyObject?
    return AXUIElementCopyAttributeValue(element, name as CFString, &out) == .success ? out : nil
}

func element(_ element: AXUIElement, _ name: String) -> AXUIElement? {
    guard let raw = value(element, name), CFGetTypeID(raw) == AXUIElementGetTypeID() else { return nil }
    return (raw as! AXUIElement)
}

func children(_ of: AXUIElement) -> [AXUIElement] {
    (value(of, kAXChildrenAttribute) as? [AXUIElement]) ?? []
}

func text(_ of: AXUIElement, _ name: String) -> String {
    (value(of, name) as? String) ?? ""
}

/// Poll `probe` every 100 ms for up to `seconds`.
func wait<T>(_ seconds: Double, _ what: String, _ probe: () -> T?) -> T {
    let deadline = Date().addingTimeInterval(seconds)
    while Date() < deadline {
        if let found = probe() { return found }
        usleep(100_000)
    }
    fail("timed out after \(seconds) s waiting for \(what)")
}

let args = CommandLine.arguments

// A template status item is drawn in one colour (white on a dark menu bar, black on a light one):
// its strokes are the pixels far from the median brightness. A foreign colour icon shows as blue.
if args.count == 3 && args[1] == "ink" {
    guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: args[2]) as CFURL, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
    else { fail("cannot read \(args[2])") }
    let width = image.width, height = image.height
    var pixels = [UInt8](repeating: 0, count: width * height * 4)
    guard let context = CGContext(data: &pixels, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                                  space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { fail("cannot draw \(args[2])") }
    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    var luma = [Double]()
    var blue = 0
    for i in stride(from: 0, to: pixels.count, by: 4) {
        let r = Double(pixels[i]), g = Double(pixels[i + 1]), b = Double(pixels[i + 2])
        luma.append((0.299 * r + 0.587 * g + 0.114 * b) / 255)
        if b > r + 60 && b > g + 40 { blue += 1 }
    }
    let median = luma.sorted()[luma.count / 2]
    let ink = luma.filter { abs($0 - median) > 0.35 }.count
    let total = Double(max(luma.count, 1))
    print(String(format: "ink=%.3f blue=%.3f size=%dx%d", Double(ink) / total, Double(blue) / total, width, height))
    exit(0)
}

guard args.count >= 3, let pid = pid_t(args[1]) else {
    fail("usage: tray-ax <pid> frame|menu|press <title>|windows|close <title>|dialog <name>")
}
guard AXIsProcessTrusted() else { fail("this binary has no Accessibility permission") }
let app = AXUIElementCreateApplication(pid)
_ = AXUIElementSetMessagingTimeout(app, 5)

func statusItem() -> AXUIElement {
    wait(15, "the status item") { () -> AXUIElement? in
        guard let bar = element(app, "AXExtrasMenuBar") else { return nil }
        return children(bar).first
    }
}

/// Press the status item and return its menu's entries (separators left out).
func openMenu() -> [AXUIElement] {
    let item = statusItem()
    let pressed = AXUIElementPerformAction(item, kAXPressAction as CFString)
    guard pressed == .success else { fail("pressing the status item failed (\(pressed.rawValue))") }
    return wait(5, "the status item's menu") { () -> [AXUIElement]? in
        guard let menu = children(item).first(where: { text($0, kAXRoleAttribute) == kAXMenuRole }) else { return nil }
        let entries = children(menu).filter { text($0, kAXRoleAttribute) == kAXMenuItemRole && !text($0, kAXTitleAttribute).isEmpty }
        return entries.isEmpty ? nil : entries
    }
}

func windows() -> [AXUIElement] {
    (value(app, kAXWindowsAttribute) as? [AXUIElement]) ?? []
}

/// Breadth-first through `root`'s tree (bounded) for an element `match` accepts.
func find(_ root: AXUIElement, _ match: (AXUIElement) -> Bool) -> AXUIElement? {
    var queue = [root]
    var seen = 0
    while !queue.isEmpty && seen < 40_000 {
        let next = queue.removeFirst()
        seen += 1
        if match(next) { return next }
        queue.append(contentsOf: children(next))
    }
    return nil
}

switch args[2] {
case "frame":
    let item = statusItem()
    var point = CGPoint.zero
    var size = CGSize.zero
    guard let position = value(item, kAXPositionAttribute), let extent = value(item, kAXSizeAttribute),
          AXValueGetValue(position as! AXValue, .cgPoint, &point), AXValueGetValue(extent as! AXValue, .cgSize, &size)
    else { fail("the status item has no frame") }
    print("\(Int(point.x)) \(Int(point.y)) \(Int(size.width)) \(Int(size.height))")
case "menu":
    let entries = openMenu()
    for entry in entries { print(text(entry, kAXTitleAttribute)) }
    if let menu = children(statusItem()).first(where: { text($0, kAXRoleAttribute) == kAXMenuRole }) {
        _ = AXUIElementPerformAction(menu, kAXCancelAction as CFString)
    }
case "press":
    guard args.count == 4 else { fail("press needs a title") }
    let entries = openMenu()
    guard let entry = entries.first(where: { text($0, kAXTitleAttribute) == args[3] }) else {
        fail("no entry '\(args[3])' among \(entries.map { text($0, kAXTitleAttribute) })")
    }
    let chosen = AXUIElementPerformAction(entry, kAXPressAction as CFString)
    guard chosen == .success else { fail("choosing '\(args[3])' failed (\(chosen.rawValue))") }
case "windows":
    for window in windows() { print(text(window, kAXTitleAttribute)) }
case "close":
    guard args.count == 4 else { fail("close needs a title") }
    guard let window = windows().first(where: { text($0, kAXTitleAttribute) == args[3] }),
          let button = element(window, kAXCloseButtonAttribute)
    else { fail("no window '\(args[3])' with a close button") }
    let closed = AXUIElementPerformAction(button, kAXPressAction as CFString)
    guard closed == .success else { fail("closing '\(args[3])' failed (\(closed.rawValue))") }
case "dialog":
    guard args.count == 4 else { fail("dialog needs a name") }
    let name = args[3]
    let found = wait(20, "a dialog named '\(name)'") { () -> AXUIElement? in
        for window in windows() {
            if let dialog = find(window, { node in
                text(node, kAXSubroleAttribute) == "AXApplicationDialog"
                    && (text(node, kAXDescriptionAttribute) == name || text(node, kAXTitleAttribute) == name)
            }) { return dialog }
        }
        return nil
    }
    print("\(text(found, kAXRoleAttribute))/\(text(found, kAXSubroleAttribute)) '\(name)'")
default:
    fail("unknown command \(args[2])")
}
