#!/usr/bin/python3 -I
"""The window the pastes land in, for scripts/repro-gnome-remote-desktop.sh (docs/dictation.md
§14.9): a GTK 4 text field that writes what it holds to OUT after every change, so the script can
tell that the keys of each paste arrived; each change, and whether its window has the keyboard, with
the time to OUT.events.

  target.py OUT
"""

import sys
import time

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gio, Gtk  # noqa: E402


def main():
    out = sys.argv[1]
    # One process per run: after GNOME Shell restarts, a new one starts while the old may still go.
    app = Gtk.Application(application_id="dev.voltip.ReproTarget", flags=Gio.ApplicationFlags.NON_UNIQUE)

    def activate(app):
        window = Gtk.ApplicationWindow(application=app, title="Voltip repro target")
        entry = Gtk.Entry()
        # The field at the top; below it a label that fills the rest of the screen, where seat.py
        # clicks: the window gets the keyboard and the field keeps its cursor at the end.
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        box.append(entry)
        rest = Gtk.Label(label="Voltip repro target")
        rest.set_vexpand(True)
        box.append(rest)

        def event(what):
            with open(out + ".events", "a", encoding="utf-8") as f:
                f.write(f"{time.time():.6f} {what}\n")

        def changed(entry):
            with open(out, "w", encoding="utf-8") as f:
                f.write(entry.get_text())
            event(f"text {entry.get_text()!r}")

        def focus(window, _param):
            event("active" if window.is_active() else "inactive")

        entry.connect("changed", changed)
        window.connect("notify::is-active", focus)
        window.set_child(box)
        window.maximize()
        window.present()
        entry.grab_focus()
        changed(entry)  # the empty field: the window is up
        focus(window, None)

    app.connect("activate", activate)
    app.run([])


if __name__ == "__main__":
    main()
