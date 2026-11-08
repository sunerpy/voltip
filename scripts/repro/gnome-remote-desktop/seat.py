#!/usr/bin/python3 -I
"""The keyboard and mouse of the session, for scripts/repro-gnome-remote-desktop.sh
(docs/dictation.md §14.9). A desktop has them all the time; headless GNOME Shell has none. Without a
keyboard in the seat no window holds the keyboard: each paste's own keyboard (the portal's, or
Xwayland's) would be the first, the text field would bind it only after its first keys were gone,
and lose it again when the paste ends. This process holds a session of Mutter's own remote-desktop
interface (org.gnome.Mutter.RemoteDesktop, which xdg-desktop-portal-gnome uses for the portal): it
asks nobody, shows no window, and is not counted in anything the scenario reports. Mutter makes its
devices on their first use, so it presses Shift once (the keyboard) and clicks (the mouse).

On start and on every SIGUSR1 it clicks the middle of the screen, as a person clicks into the
window they are about to dictate into: target.py's window fills the screen. SIGTERM ends it.

  seat.py WIDTH HEIGHT READY
Writes READY once the session runs and the first click is done.
"""

import signal
import sys

import gi

gi.require_version("Gio", "2.0")
gi.require_version("GLibUnix", "2.0")
from gi.repository import Gio, GLib, GLibUnix  # noqa: E402

BTN_LEFT = 0x110  # linux/input-event-codes.h
SHIFT_L = 0xFFE1  # the keysym
SESSION = "org.gnome.Mutter.RemoteDesktop.Session"


def main():
    width, height, ready = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    bus = Gio.bus_get_sync(Gio.BusType.SESSION)

    def call(path, interface, method, args=None):
        return bus.call_sync("org.gnome.Mutter.RemoteDesktop", path, interface, method, args, None,
                             Gio.DBusCallFlags.NONE, 5000, None)

    (session,) = call("/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop", "CreateSession").unpack()
    call(session, SESSION, "Start")

    def click():
        # To the top-left corner wherever the pointer is, then to the middle of the screen.
        call(session, SESSION, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (-4.0 * width, -4.0 * height)))
        call(session, SESSION, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (width / 2, height / 2)))
        call(session, SESSION, "NotifyPointerButton", GLib.Variant("(ib)", (BTN_LEFT, True)))
        call(session, SESSION, "NotifyPointerButton", GLib.Variant("(ib)", (BTN_LEFT, False)))

    loop = GLib.MainLoop()

    def on_usr1():
        click()
        return GLib.SOURCE_CONTINUE

    def on_term():
        loop.quit()
        return GLib.SOURCE_REMOVE

    GLibUnix.signal_add(GLib.PRIORITY_DEFAULT, signal.SIGUSR1, on_usr1)
    GLibUnix.signal_add(GLib.PRIORITY_DEFAULT, signal.SIGTERM, on_term)
    call(session, SESSION, "NotifyKeyboardKeysym", GLib.Variant("(ub)", (SHIFT_L, True)))
    call(session, SESSION, "NotifyKeyboardKeysym", GLib.Variant("(ub)", (SHIFT_L, False)))
    click()
    with open(ready, "w", encoding="utf-8") as f:
        f.write("ready\n")
    loop.run()
    try:
        call(session, SESSION, "Stop")
    except GLib.Error:
        pass  # GNOME Shell already gone


if __name__ == "__main__":
    main()
