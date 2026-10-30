#!/usr/bin/python3
"""A GTK 3 window for tests/focus_linux.rs (docs/dictation.md §28): an editable entry, a read-only
one and a second editable one, each holding "前文" with the caret at its end.

stdin takes one command per line: an entry's name ("editable", "readonly", "other") gives it the
focus and answers "focused <name>"; "quit" ends. "ready" is printed once the window is mapped.
"""

import sys

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk  # noqa: E402

TITLE = "voltip-focus-check"

window = Gtk.Window(title=TITLE)
box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
entries = {}
for name, editable in (("editable", True), ("readonly", False), ("other", True)):
    entry = Gtk.Entry()
    entry.set_text("前文")
    entry.set_editable(editable)
    box.pack_start(entry, False, False, 0)
    entries[name] = entry
window.add(box)
window.connect("destroy", Gtk.main_quit)


def say(line):
    print(line, flush=True)
    return False


def on_command(source, condition):
    line = sys.stdin.readline()
    command = line.strip()
    if not line or command == "quit":
        Gtk.main_quit()
        return False
    entry = entries.get(command)
    if entry is not None:
        window.present()
        entry.grab_focus()
        # grab_focus selects the whole text; the caret goes to the end with nothing selected.
        entry.set_position(-1)
        GLib.timeout_add(150, say, f"focused {command}")
    return True


GLib.io_add_watch(sys.stdin, GLib.IO_IN | GLib.IO_HUP, on_command)
window.connect("map-event", lambda *_: GLib.timeout_add(300, say, "ready") and False)
window.show_all()
Gtk.main()
