// Drive another process's menu bar status item through the Accessibility API, for
// scripts/smoke-tray-macos.sh. Needs the Accessibility permission (the smoke grants it to the
// compiled binary in the runner's TCC database).
//
//   tray-ax <pid> frame              x y width height of the status item (points)
//   tray-ax <pid> menu               open the item's menu, print its entries, close it
//   tray-ax <pid> press <title>      open the menu and choose the entry titled <title>
//   tray-ax <pid> submenu <title>    open the submenu of the entry <title>, print its entries
//                                    (a checked one as "<entry><TAB>✓"), close the menu
//   tray-ax <pid> press-sub <title> <entry>  open that submenu and choose <entry>
//   tray-ax <pid> click              a real left click at the item's centre (mouse events, not an
//                                    accessibility press); waits for a menu of the app to open,
//                                    prints how many milliseconds after the click it did, closes it
//   tray-ax <pid> doubleclick <ms>   a real double click at the item's centre; then "menu" when a
//                                    menu of the app opened within <ms> (and closes it), else
//                                    "no menu"
//   tray-ax <pid> windows            the titles of the process's windows, one per line
//   tray-ax <pid> close <title>      press the close button of the window titled <title>
//   tray-ax <pid> dialog <name>      exit 0 when a window holds a dialog named <name>
//                                    (a webview's role=dialog: AXApplicationDialog)
//   tray-ax <pid> wait-text <title> <text>  wait until the window titled <title> shows <text>
//                                    (a static text or heading of its webview): it has drawn
//   tray-ax <pid> chrome <title>     "x y width height zoom-right" of the window titled <title>:
//                                    its frame and where its green (zoom) button ends (points)
//   tray-ax ink <png>                "ink=<share> blue=<share>": the pixels that stand out from
//                                    the image's median brightness, and the saturated blue ones
//   tray-ax gap <png> <x>            "gap=<px>": from column <x> of a picture of the title bar's
//                                    left end, how far right the first drawn pixel of the
//                                    window's content is (the traffic lights end at <x>)
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

/// The RGBA pixels of a PNG, row-major from the top.
func rgba(_ file: String) -> (pixels: [UInt8], width: Int, height: Int) {
    guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: file) as CFURL, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
    else { fail("cannot read \(file)") }
    let width = image.width, height = image.height
    var pixels = [UInt8](repeating: 0, count: width * height * 4)
    guard let context = CGContext(data: &pixels, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                                  space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { fail("cannot draw \(file)") }
    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    return (pixels, width, height)
}

// Past the traffic lights, the first column in the strip's middle band (a quarter to three
// quarters of its height) that holds a pixel unlike the bar's background, taken from the four
// columns right after <x>, where nothing is drawn in either layout.
if args.count == 4 && args[1] == "gap", let from = Int(args[3]) {
    let (pixels, width, height) = rgba(args[2])
    guard from + 4 < width else { fail("column \(from) is past the picture (\(width) px)") }
    let rows = (height / 4)..<(height * 3 / 4)
    func px(_ x: Int, _ y: Int) -> (Double, Double, Double) {
        let i = (y * width + x) * 4
        return (Double(pixels[i]), Double(pixels[i + 1]), Double(pixels[i + 2]))
    }
    var background = (0.0, 0.0, 0.0)
    var samples = 0.0
    for x in (from + 1)...(from + 4) {
        for y in rows {
            let p = px(x, y)
            background = (background.0 + p.0, background.1 + p.1, background.2 + p.2)
            samples += 1
        }
    }
    background = (background.0 / samples, background.1 / samples, background.2 / samples)
    for x in (from + 1)..<width {
        for y in rows {
            let p = px(x, y)
            if abs(p.0 - background.0) + abs(p.1 - background.1) + abs(p.2 - background.2) > 60 {
                print("gap=\(x - from)")
                exit(0)
            }
        }
    }
    fail("nothing drawn after column \(from)")
}

// A template status item is drawn in one colour (white on a dark menu bar, black on a light one):
// its strokes are the pixels far from the median brightness. A foreign colour icon shows as blue.
if args.count == 3 && args[1] == "ink" {
    let (pixels, width, height) = rgba(args[2])
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
    fail("usage: tray-ax <pid> frame|menu|press <title>|submenu <title>|press-sub <title> <entry>|click|doubleclick <ms>|windows|close <title>|chrome <title>|dialog <name>")
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

/// The status item's menu. The accessibility tree carries it whether or not it is open (as it
/// carries a menu bar's menus), so it says nothing about what the screen shows: `watchMenus`
/// does.
func openedMenu(_ item: AXUIElement) -> AXUIElement? {
    children(item).first(where: { text($0, kAXRoleAttribute) == kAXMenuRole })
}

/// The first menu of the app that opened since `watchMenus`, and when (system uptime).
var menuOpened: (menu: AXUIElement, at: TimeInterval)?

/// Start listening for the app's AXMenuOpened notifications (the one VoiceOver announces a menu
/// by); keep the observer until the listening is done.
func watchMenus() -> AXObserver {
    var made: AXObserver?
    let created = AXObserverCreate(pid, { _, element, _, _ in
        if menuOpened == nil { menuOpened = (element, ProcessInfo.processInfo.systemUptime) }
    }, &made)
    guard created == .success, let observer = made else { fail("cannot observe the app (\(created.rawValue))") }
    let added = AXObserverAddNotification(observer, app, kAXMenuOpenedNotification as CFString, nil)
    guard added == .success else { fail("cannot listen for the app's menus (\(added.rawValue))") }
    CFRunLoopAddSource(CFRunLoopGetCurrent(), AXObserverGetRunLoopSource(observer), .defaultMode)
    return observer
}

/// Run the run loop (which delivers the notifications) until a menu opened or `seconds` passed.
func awaitMenu(_ seconds: Double) {
    let deadline = Date().addingTimeInterval(seconds)
    while menuOpened == nil && Date() < deadline {
        _ = CFRunLoopRunInMode(.defaultMode, 0.05, true)
    }
}

/// Left-button mouse events at the status item's centre, as a hand makes them (the cursor moves
/// there): `presses` clicks, the n-th carrying click count n, so AppKit sees a double click.
/// `beforeLastUp` runs just before the last button-up is posted. The pauses are a hand's: a press
/// held 40 ms, and 100 ms from one release to the next, well inside the shortest double-click
/// interval System Settings offers.
func clickItem(_ item: AXUIElement, presses: Int, beforeLastUp: () -> Void = {}) {
    let (x, y, w, h) = frame(item)
    let at = CGPoint(x: Double(x) + Double(w) / 2, y: Double(y) + Double(h) / 2)
    func post(_ type: CGEventType, _ count: Int) {
        guard let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: at, mouseButton: .left)
        else { fail("cannot make a mouse event") }
        event.setIntegerValueField(.mouseEventClickState, value: Int64(count))
        event.post(tap: .cghidEventTap)
    }
    post(.mouseMoved, 0)
    usleep(100_000)
    for n in 1...presses {
        post(.leftMouseDown, n)
        usleep(40_000)
        if n == presses { beforeLastUp() }
        post(.leftMouseUp, n)
        if n < presses { usleep(60_000) }
    }
}

/// Press the status item and return its menu's entries (separators left out). The press returns
/// only when the menu closes (AppKit tracks it in a modal loop), so the call times out with
/// kAXErrorCannotComplete while the menu is open: that answer counts as opened, and the menu is
/// then read like any other element.
func openMenu() -> [AXUIElement] {
    let item = statusItem()
    _ = AXUIElementSetMessagingTimeout(item, 1)
    let pressed = AXUIElementPerformAction(item, kAXPressAction as CFString)
    guard pressed == .success || pressed == .cannotComplete else { fail("pressing the status item failed (\(pressed.rawValue))") }
    return wait(5, "the status item's menu") { () -> [AXUIElement]? in
        guard let menu = openedMenu(item) else { return nil }
        let entries = children(menu).filter { text($0, kAXRoleAttribute) == kAXMenuItemRole && !text($0, kAXTitleAttribute).isEmpty }
        return entries.isEmpty ? nil : entries
    }
}

/// Open the status item's menu, then the submenu of its entry `title`, and return the submenu's
/// entries (separators left out). Pressing an entry that has a submenu opens it.
func openSubmenu(_ title: String) -> [AXUIElement] {
    let entries = openMenu()
    guard let parent = entries.first(where: { text($0, kAXTitleAttribute) == title }) else {
        fail("no entry '\(title)' among \(entries.map { text($0, kAXTitleAttribute) })")
    }
    _ = AXUIElementSetMessagingTimeout(parent, 1)
    let pressed = AXUIElementPerformAction(parent, kAXPressAction as CFString)
    guard pressed == .success || pressed == .cannotComplete else { fail("opening '\(title)' failed (\(pressed.rawValue))") }
    return wait(5, "the submenu '\(title)'") { () -> [AXUIElement]? in
        guard let menu = children(parent).first(where: { text($0, kAXRoleAttribute) == kAXMenuRole }) else { return nil }
        let items = children(menu).filter { text($0, kAXRoleAttribute) == kAXMenuItemRole && !text($0, kAXTitleAttribute).isEmpty }
        return items.isEmpty ? nil : items
    }
}

/// Close whatever menu the status item has open (a submenu closes with it).
func cancelMenu() {
    if let menu = openedMenu(statusItem()) {
        _ = AXUIElementPerformAction(menu, kAXCancelAction as CFString)
    }
}

func windows() -> [AXUIElement] {
    (value(app, kAXWindowsAttribute) as? [AXUIElement]) ?? []
}

/// An element's frame in whole points, top-left origin.
func frame(_ of: AXUIElement) -> (Int, Int, Int, Int) {
    var point = CGPoint.zero
    var size = CGSize.zero
    guard let position = value(of, kAXPositionAttribute), let extent = value(of, kAXSizeAttribute),
          AXValueGetValue(position as! AXValue, .cgPoint, &point), AXValueGetValue(extent as! AXValue, .cgSize, &size)
    else { fail("the element has no frame") }
    return (Int(point.x), Int(point.y), Int(size.width), Int(size.height))
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
    let (x, y, w, h) = frame(statusItem())
    print("\(x) \(y) \(w) \(h)")
case "menu":
    let entries = openMenu()
    for entry in entries { print(text(entry, kAXTitleAttribute)) }
    cancelMenu()
case "press":
    guard args.count == 4 else { fail("press needs a title") }
    let entries = openMenu()
    guard let entry = entries.first(where: { text($0, kAXTitleAttribute) == args[3] }) else {
        fail("no entry '\(args[3])' among \(entries.map { text($0, kAXTitleAttribute) })")
    }
    let chosen = AXUIElementPerformAction(entry, kAXPressAction as CFString)
    guard chosen == .success else { fail("choosing '\(args[3])' failed (\(chosen.rawValue))") }
case "submenu":
    // One line per entry; a checked one ends in a tab and its mark (✓).
    guard args.count == 4 else { fail("submenu needs the parent entry's title") }
    for entry in openSubmenu(args[3]) {
        let mark = text(entry, kAXMenuItemMarkCharAttribute)
        print(mark.isEmpty ? text(entry, kAXTitleAttribute) : "\(text(entry, kAXTitleAttribute))\t\(mark)")
    }
    cancelMenu()
case "press-sub":
    guard args.count == 5 else { fail("press-sub needs the parent entry's title and the entry's") }
    let entries = openSubmenu(args[3])
    guard let entry = entries.first(where: { text($0, kAXTitleAttribute) == args[4] }) else {
        fail("no entry '\(args[4])' among \(entries.map { text($0, kAXTitleAttribute) })")
    }
    let chosen = AXUIElementPerformAction(entry, kAXPressAction as CFString)
    guard chosen == .success else { fail("choosing '\(args[4])' failed (\(chosen.rawValue))") }
case "click":
    // Timed from just before the release the app answers, so the figure is never shorter than the
    // app's wait.
    let item = statusItem()
    let observer = watchMenus()
    var released = 0.0
    clickItem(item, presses: 1) { released = ProcessInfo.processInfo.systemUptime }
    awaitMenu(15)
    guard let opened = menuOpened else { fail("no menu opened within 15 s of a click") }
    print(Int((opened.at - released) * 1000))
    _ = AXUIElementPerformAction(opened.menu, kAXCancelAction as CFString)
    withExtendedLifetime(observer) {}
case "doubleclick":
    guard args.count == 4, let watch = Double(args[3]) else { fail("doubleclick needs how many ms to watch for a menu") }
    let observer = watchMenus()
    clickItem(statusItem(), presses: 2)
    awaitMenu(watch / 1000)
    if let opened = menuOpened {
        _ = AXUIElementPerformAction(opened.menu, kAXCancelAction as CFString)
        print("menu")
    } else {
        print("no menu")
    }
    withExtendedLifetime(observer) {}
case "windows":
    for window in windows() { print(text(window, kAXTitleAttribute)) }
case "wait-text":
    guard args.count == 5 else { fail("wait-text needs a window title and a text") }
    let (title, wanted) = (args[3], args[4])
    let found = wait(30, "'\(wanted)' in the window '\(title)'") { () -> AXUIElement? in
        guard let window = windows().first(where: { text($0, kAXTitleAttribute) == title }) else { return nil }
        return find(window) { node in
            let role = text(node, kAXRoleAttribute)
            return (role == kAXStaticTextRole || role == "AXHeading")
                && (text(node, kAXValueAttribute) == wanted || text(node, kAXTitleAttribute) == wanted)
        }
    }
    let (x, y, w, h) = frame(found)
    print("\(x) \(y) \(w) \(h)")
case "chrome":
    guard args.count == 4 else { fail("chrome needs a title") }
    guard let window = windows().first(where: { text($0, kAXTitleAttribute) == args[3] }),
          let zoom = element(window, kAXZoomButtonAttribute)
    else { fail("no window '\(args[3])' with a zoom button") }
    let (wx, wy, ww, wh) = frame(window)
    let (zx, _, zw, _) = frame(zoom)
    print("\(wx) \(wy) \(ww) \(wh) \(zx + zw)")
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
