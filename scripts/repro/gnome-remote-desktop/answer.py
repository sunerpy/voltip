#!/usr/bin/python3 -I
"""Answers GNOME's remote-desktop window the way a person would, and counts the windows.

Part of scripts/repro-gnome-remote-desktop.sh (docs/dictation.md §14.9, issue #235). Watches the
accessibility tree of the session for the window xdg-desktop-portal-gnome opens ("Remote Desktop")
and answers each as the file ANSWER says at that moment:

  allow   switch "Allow Remote Interaction" on, leave "Remember This Selection" as it is, Share
  cancel  Cancel

Each answered window is one line in LOG: `window <n> <answer> remember=<hidden|ticked|unticked>`.
A window it cannot answer is described on stderr, every node with its role, name, states and
actions, so the script can be fixed. Runs until it is killed (the session script stops it).

  answer.py ANSWER LOG
"""

import sys
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, GLib  # noqa: E402

TITLE = "Remote Desktop"
SWITCH = "Allow Remote Interaction"
REMEMBER = "Remember This Selection"
# How often the tree is read. The tree's own events arrive through the main loop below; the
# portal's window has no event this script could wait on directly, so it is looked for.
POLL_MS = 100
# How long a window may take to enable Share after the switch, and to close after Share.
SETTLE_S = 10


def pump():
    """Let the accessibility library take the events that arrived (the tree's cache)."""
    context = GLib.MainContext.default()
    while context.pending():
        context.iteration(False)


def walk(node, depth=0):
    """Every accessible below `node`, depth first (bounded: the window is small)."""
    if node is None or depth > 40:
        return
    yield node
    try:
        count = node.get_child_count()
    except Exception:  # an object that went away while we looked
        return
    for i in range(count):
        try:
            child = node.get_child_at_index(i)
        except Exception:
            continue
        yield from walk(child, depth + 1)


def name_of(node):
    try:
        return (node.get_name() or "").replace("_", "")
    except Exception:
        return ""


def role_of(node):
    try:
        return node.get_role()
    except Exception:
        return None


def has(node, state):
    try:
        return node.get_state_set().contains(state)
    except Exception:
        return False


def actions_of(node):
    try:
        action = node.get_action_iface()
    except Exception:
        return None, []
    if action is None:
        return None, []
    names = []
    try:
        count = action.get_n_actions()
    except Exception:
        return None, []
    for i in range(count):
        try:
            names.append(action.get_action_name(i))
        except Exception:  # some widgets count actions they cannot name
            names.append("")
    return action, names


def act(node, preferred):
    """Run the first of the node's actions named in `preferred`, else its first action."""
    action, names = actions_of(node)
    if action is None or not names:
        return False
    for wanted in preferred:
        if wanted in names:
            return action.do_action(names.index(wanted))
    return action.do_action(0)


def portal_windows():
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(i)
        if app is None or "portal" not in name_of(app).lower():
            continue
        for j in range(app.get_child_count()):
            window = app.get_child_at_index(j)
            if window is not None and name_of(window) == TITLE and has(window, Atspi.StateType.SHOWING):
                yield window


def find(window, name, roles):
    """The node called `name` in one of `roles`; one that has actions first. GTK 4 shows a row and
    the widget in it under the same name ("Allow Remote Interaction": the AdwSwitchRow, and the
    GtkSwitch in it that toggles)."""
    found = None
    for node in walk(window):
        if name_of(node) == name and role_of(node) in roles:
            if any(actions_of(node)[1]):
                return node
            if found is None:
                found = node
    return found


def describe(window):
    for node in walk(window):
        role = role_of(node)
        states = []
        for state in (Atspi.StateType.SHOWING, Atspi.StateType.SENSITIVE, Atspi.StateType.CHECKED):
            if has(node, state):
                states.append(state.value_nick)
        _, actions = actions_of(node)
        nick = role.value_nick if role is not None else "?"
        print(f"  {nick} {name_of(node)!r} states={states} actions={actions}", file=sys.stderr, flush=True)


def wait_for(condition):
    deadline = time.monotonic() + SETTLE_S
    while time.monotonic() < deadline:
        pump()
        if condition():
            return True
        time.sleep(POLL_MS / 1000)
    return False


def answer(window, mode):
    """Answer one window; returns (answered, what "Remember This Selection" showed)."""
    remember = find(window, REMEMBER, (Atspi.Role.CHECK_BOX, Atspi.Role.TOGGLE_BUTTON))
    if remember is None or not has(remember, Atspi.StateType.SHOWING):
        kept = "hidden"
    else:
        kept = "ticked" if has(remember, Atspi.StateType.CHECKED) else "unticked"
    if mode == "cancel":
        button = find(window, "Cancel", (Atspi.Role.BUTTON,))
        return button is not None and act(button, ("click", "activate")), kept
    # The row by its title; else the first switch on screen (the clipboard row is hidden: Voltip
    # asks for the keyboard only).
    switch = find(window, SWITCH, (Atspi.Role.SWITCH, Atspi.Role.TOGGLE_BUTTON, Atspi.Role.LIST_ITEM))
    if switch is None or not any(actions_of(switch)[1]):
        for node in walk(window):
            if role_of(node) == Atspi.Role.SWITCH and has(node, Atspi.StateType.SHOWING) and any(actions_of(node)[1]):
                switch = node
                break
    if switch is None:
        return False, kept
    if not has(switch, Atspi.StateType.CHECKED):
        act(switch, ("toggle", "activate", "click"))

    def share_ready():
        share = find(window, "Share", (Atspi.Role.BUTTON,))
        return share is not None and has(share, Atspi.StateType.SENSITIVE)

    if not wait_for(share_ready):
        return False, kept
    share = find(window, "Share", (Atspi.Role.BUTTON,))
    return share is not None and act(share, ("click", "activate")), kept


def main():
    answer_file, log_file = sys.argv[1], sys.argv[2]
    answered = 0
    failed = 0
    loop = GLib.MainLoop()

    def tick():
        # An exception would remove this source from the loop: report it and keep looking.
        try:
            look()
        except Exception as e:
            print(f"answer.py: {e!r}", file=sys.stderr, flush=True)
        return GLib.SOURCE_CONTINUE

    def look():
        nonlocal answered, failed
        try:
            windows = list(portal_windows())
        except Exception:
            windows = []
        if not windows:
            return
        window = windows[0]
        try:
            with open(answer_file, encoding="utf-8") as f:
                mode = f.read().strip() or "allow"
        except OSError:
            mode = "allow"
        done, kept = answer(window, mode)
        if not done:
            failed += 1
            if failed in (1, 50):  # once at first, once more if it keeps failing
                print(f"answer.py: could not answer the window ({mode}):", file=sys.stderr, flush=True)
                describe(window)
            return
        failed = 0
        answered += 1
        with open(log_file, "a", encoding="utf-8") as log:
            log.write(f"window {answered} {mode} remember={kept}\n")
        # One window is one request: wait for it to close before looking for the next.
        if not wait_for(lambda: not any(True for _ in portal_windows())):
            print(f"answer.py: window {answered} still open {SETTLE_S} s after {mode}", file=sys.stderr, flush=True)
        return

    GLib.timeout_add(POLL_MS, tick)
    loop.run()


if __name__ == "__main__":
    main()
