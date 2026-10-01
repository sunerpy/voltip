// 「粘贴到上一个窗口」 (`paste_text`, `voltip_core::paste`): the Tauri backend's call and answer
// check, and the mock's copy of the shell's and the core's refusals.
import { MockBackend } from "./mock-backend";
import { MAX_PASTE_TEXT_CHARS, pasteOutcomeSchema } from "./schema";
import { TauriBackend } from "./tauri-backend";

describe("paste_text", () => {
  it("invokes paste_text with the text and validates the answer", async () => {
    const calls: { command: string; args: unknown }[] = [];
    let answer: unknown = { kind: "copied", reason: "target_changed" };
    const backend = new TauriBackend({
      invoke: (command, args) => {
        calls.push({ command, args });
        return Promise.resolve(answer);
      },
      listen: () => Promise.resolve(() => undefined),
    });
    expect(await backend.pasteText("你好")).toEqual({ kind: "copied", reason: "target_changed" });
    expect(calls).toEqual([{ command: "paste_text", args: { text: "你好" } }]);
    answer = { kind: "failed", reason: "sometimes" };
    await expect(backend.pasteText("你好")).rejects.toThrow(/invalid|expected/i);
    answer = { kind: "pasted", reason: "busy" };
    expect(pasteOutcomeSchema.parse(answer)).toEqual({ kind: "pasted" });
  });

  it("the mock refuses what the shell and the core refuse, and records the rest", async () => {
    const backend = new MockBackend({ now: () => 1_758_700_000_000 });
    const history = backend.peek().history_recent;
    expect(await backend.pasteText("你好")).toEqual({ kind: "pasted" });
    expect(await backend.pasteText(" \n")).toEqual({ kind: "failed", reason: "invalid" });
    expect(await backend.pasteText("字".repeat(MAX_PASTE_TEXT_CHARS + 1))).toEqual({
      kind: "failed",
      reason: "invalid",
    });
    expect(await backend.pasteText("字".repeat(MAX_PASTE_TEXT_CHARS))).toEqual({ kind: "pasted" });
    backend.setPasteOutcome({ kind: "copied", reason: "timeout" });
    expect(await backend.pasteText("再来")).toEqual({ kind: "copied", reason: "timeout" });
    await backend.invoke("dictation_start");
    expect(await backend.pasteText("听写中")).toEqual({ kind: "failed", reason: "busy" });
    await backend.invoke("dictation_cancel");
    expect(backend.pastes).toEqual(["你好", "字".repeat(MAX_PASTE_TEXT_CHARS), "再来"]);
    expect(backend.peek().history_recent).toEqual(history);
    // Changed by the user's request of 2026-09-30 (item 10, docs/dictation.md §20.7): the phone
    // has no window to paste into and copies to its clipboard (it answered `unsupported` before).
    expect(await new MockBackend({ role: "phone" }).pasteText("你好")).toEqual({
      kind: "copied",
      reason: "clipboard_only",
    });
  });
});
