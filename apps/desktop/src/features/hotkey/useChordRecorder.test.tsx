import { MockBackend } from "@voltip/shared/mock";
import { BackendProvider } from "@voltip/ui";
import { act, fireEvent, render, screen } from "@testing-library/react";
import {
  chordFromCodes,
  chordPartFromCode,
  chordProblem,
  useChordRecorder,
} from "./useChordRecorder";

function Probe({
  onCommit,
  onReject,
}: {
  onCommit: (c: string) => void;
  onReject: (r: string) => void;
}) {
  const rec = useChordRecorder({ onCommit, onReject });
  return (
    <button
      type="button"
      data-testid="rec"
      data-recording={rec.recording}
      data-preview={rec.preview ?? ""}
      onClick={rec.recording ? rec.cancel : rec.start}>
      rec
    </button>
  );
}

function mount() {
  const backend = new MockBackend();
  const onCommit = vi.fn();
  const onReject = vi.fn();
  render(
    <BackendProvider backend={backend}>
      <Probe onCommit={onCommit} onReject={onReject} />
    </BackendProvider>,
  );
  return { backend, onCommit, onReject, button: screen.getByTestId("rec") };
}

const key = (type: "keydown" | "keyup", code: string, extra: KeyboardEventInit = {}) => {
  act(() => {
    window.dispatchEvent(new KeyboardEvent(type, { code, key: code, ...extra }));
  });
};

describe("useChordRecorder", () => {
  it("regression: recording suspends the OS hotkey (hotkey_capture) and restores it when the chord is committed", async () => {
    const { backend, onCommit, button } = mount();
    fireEvent.click(button);
    await act(async () => {
      await Promise.resolve();
    });
    expect(button.dataset.recording).toBe("true");
    expect(backend.peek().hotkey.capturing).toBe(true);
    // Physical keys: Shift released before D still records the peak set, and the chord is canonical.
    key("keydown", "ShiftLeft");
    key("keydown", "ControlLeft");
    key("keydown", "KeyD");
    expect(button.dataset.preview).toBe("Ctrl+Shift+D");
    key("keyup", "ShiftLeft");
    await act(async () => {
      await Promise.resolve();
    });
    expect(onCommit).toHaveBeenCalledWith("Ctrl+Shift+D");
    expect(button.dataset.recording).toBe("false");
    expect(backend.peek().hotkey.capturing).toBe(false);
    // Later key-ups of the same chord are not a second commit.
    key("keyup", "ControlLeft");
    key("keyup", "KeyD");
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it("refuses modifier-only, multi-key and bare-key chords with a reason, ignores key repeat, and Esc cancels", async () => {
    const { onCommit, onReject, button } = mount();
    fireEvent.click(button);
    key("keydown", "ControlLeft");
    key("keydown", "ControlLeft", { repeat: true });
    key("keyup", "ControlLeft");
    expect(onReject).toHaveBeenLastCalledWith("只按了 Ctrl、Alt 这类键，还需要再加一个普通键");
    fireEvent.click(button);
    key("keydown", "ControlLeft");
    key("keydown", "KeyA");
    key("keydown", "KeyB");
    key("keyup", "KeyA");
    expect(onReject).toHaveBeenLastCalledWith("一次只能设一个普通键");
    fireEvent.click(button);
    key("keydown", "KeyX");
    key("keyup", "KeyX");
    expect(onReject).toHaveBeenLastCalledWith(
      "这个系统不允许只用一个键，请再加一个 Ctrl、Alt 之类的键",
    );
    fireEvent.click(button);
    key("keydown", "ControlLeft");
    key("keydown", "Escape", { key: "Escape" });
    expect(button.dataset.recording).toBe("false");
    expect(button.dataset.preview).toBe("");
    // A key-up that arrives after the recorder closed does nothing.
    key("keyup", "ControlLeft");
    expect(onCommit).not.toHaveBeenCalled();
    await act(async () => {
      await Promise.resolve();
    });
  });

  it("regression: losing the window cancels the recording so the hotkey is never left suspended", async () => {
    const { backend, button } = mount();
    fireEvent.click(button);
    await act(async () => {
      await Promise.resolve();
    });
    expect(backend.peek().hotkey.capturing).toBe(true);
    act(() => {
      window.dispatchEvent(new Event("blur"));
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(button.dataset.recording).toBe("false");
    expect(backend.peek().hotkey.capturing).toBe(false);
    expect(backend.peek().hotkey.registered).toBe(backend.peek().settings.hotkey);
  });

  it("maps physical codes onto the core's chord vocabulary", () => {
    expect(chordPartFromCode("ControlRight")).toBe("Ctrl");
    expect(chordPartFromCode("AltLeft")).toBe("Alt");
    expect(chordPartFromCode("ShiftRight")).toBe("Shift");
    expect(chordPartFromCode("MetaLeft")).toBe("Meta");
    expect(chordPartFromCode("OSLeft")).toBe("Meta");
    expect(chordPartFromCode("KeyQ")).toBe("Q");
    expect(chordPartFromCode("Digit7")).toBe("7");
    expect(chordPartFromCode("Space")).toBe("Space");
    expect(chordPartFromCode("F5")).toBe("F5");
    expect(chordPartFromCode("Comma")).toBe("Comma");
    expect(chordPartFromCode("")).toBeUndefined();
    expect(chordFromCodes(["KeyD", "MetaLeft", "ShiftLeft", "ControlLeft", "AltLeft"])).toBe(
      "Ctrl+Alt+Shift+Meta+D",
    );
    expect(chordFromCodes(["ControlLeft", "ControlRight"])).toBe("Ctrl");
    expect(chordProblem("Ctrl+Alt+Space")).toBeUndefined();
    expect(chordProblem("")).toBe("只按了 Ctrl、Alt 这类键，还需要再加一个普通键");
  });
});
