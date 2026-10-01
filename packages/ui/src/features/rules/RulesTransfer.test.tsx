import type { HistoryEntry } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import { act, render, renderHook, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { BackendProvider } from "../../backend/BackendProvider";
import { I18nProvider } from "../../i18n/I18nProvider";
import { FeatureShellProvider } from "../shell";
import { useHitTotals } from "../vocabulary/useHitTotals";
import { PREVIEW_DEBOUNCE_MS, useVocabularyPreview } from "../vocabulary/usePreview";
import { RulesExportDialog, RulesImportDialog } from "./RulesTransfer";

const TOML =
  'version = 1\n\n[[rule]]\nname = "句号"\nkind = "literal"\npattern = "。。"\nreplacement = "。"\n';

function wrap(backend: MockBackend, notify = vi.fn()) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <BackendProvider backend={backend}>
        <I18nProvider locale="zh-CN">
          <FeatureShellProvider shell={{ notify, confirm: () => undefined }}>
            {children}
          </FeatureShellProvider>
        </I18nProvider>
      </BackendProvider>
    );
  };
}

describe("the rules' TOML dialogs (desktop and phone, docs/dictation.md section 16)", () => {
  it("imports a pasted file by merge or replace and keeps the core's refusal in the dialog", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const notify = vi.fn();
    const onClose = vi.fn();
    render(<RulesImportDialog onClose={onClose} />, { wrapper: wrap(backend, notify) });
    const dialog = screen.getByRole("dialog", { name: "导入 TOML" });
    expect(within(dialog).getByRole("button", { name: "导入" })).toBeDisabled();
    await user.click(within(dialog).getByLabelText("TOML 文本"));
    await user.paste("not toml");
    await user.click(within(dialog).getByRole("button", { name: "导入" }));
    expect(await within(dialog).findByTestId("rules-import-error")).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
    await user.clear(within(dialog).getByLabelText("TOML 文本"));
    expect(within(dialog).queryByTestId("rules-import-error")).toBeNull();
    await user.click(within(dialog).getByLabelText("TOML 文本"));
    await user.paste(TOML);
    await user.click(within(dialog).getByRole("radio", { name: "替换" }));
    expect(within(dialog).getByText("文件里的规则成为全部规则")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "导入" }));
    await waitFor(() => {
      expect(backend.peek().rules.map((r) => r.name)).toEqual(["句号"]);
    });
    expect(notify).toHaveBeenCalledWith("已导入 · 替换");
    expect(onClose).toHaveBeenCalled();
    backend.destroy();
  });

  it("shows the exported text with the app's own buttons after 关闭", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const onClose = vi.fn();
    render(
      <RulesExportDialog
        text={TOML}
        onClose={onClose}
        actions={<button type="button">share it</button>}
      />,
      { wrapper: wrap(backend) },
    );
    const dialog = screen.getByRole("dialog", { name: "导出 TOML" });
    expect(within(dialog).getByTestId("rules-export-text")).toHaveValue(TOML);
    expect(within(dialog).getByRole("button", { name: "share it" })).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "关闭" }));
    expect(onClose).toHaveBeenCalled();
    backend.destroy();
  });
});

describe("the vocabulary hooks (desktop and phone)", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("previews a text after the typing pauses, reports a refusal, and stays idle on an empty text", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const backend = new MockBackend();
    await act(() =>
      backend.invoke("rules_add", {
        rule: {
          name: "嗯",
          kind: "literal",
          pattern: "嗯",
          replacement: "",
          case_sensitive: true,
          enabled: true,
        },
      }),
    );
    const { result, rerender } = renderHook(
      ({ text, allowEmpty }: { text: string; allowEmpty: boolean }) =>
        useVocabularyPreview(text, undefined, allowEmpty),
      { wrapper: wrap(backend), initialProps: { text: "", allowEmpty: false } },
    );
    expect(result.current).toEqual({ kind: "idle" });
    rerender({ text: "嗯好", allowEmpty: false });
    await act(() => vi.advanceTimersByTimeAsync(PREVIEW_DEBOUNCE_MS));
    await waitFor(() => {
      expect(result.current.kind === "ok" && result.current.preview.output).toBe("好");
    });
    vi.spyOn(backend, "vocabularyPreview").mockRejectedValueOnce(new Error("vocabulary: 坏了"));
    rerender({ text: "嗯嗯", allowEmpty: false });
    await act(() => vi.advanceTimersByTimeAsync(PREVIEW_DEBOUNCE_MS));
    await waitFor(() => {
      expect(result.current).toEqual({ kind: "error", message: "坏了" });
    });
    // An empty text is only sent when asked (the rule editor checks a draft that way).
    rerender({ text: "", allowEmpty: true });
    await act(() => vi.advanceTimersByTimeAsync(PREVIEW_DEBOUNCE_MS));
    await waitFor(() => {
      expect(result.current.kind).toBe("ok");
    });
    backend.destroy();
  });

  it("sums how often each entry and rule fired over the history, and none when the query fails", async () => {
    const entry = (vocabulary: HistoryEntry["vocabulary"]): HistoryEntry => ({
      id: "00000000-0000-4000-8000-000000000001",
      at_ms: 1,
      raw_text: "",
      text: "",
      refined: false,
      asr_model: "m",
      duration_ms: 1,
      asr_ms: 1,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      mode: "whole_take",
      kind: "dictation",
      vocabulary,
    });
    const backend = new MockBackend({
      history: [entry({ corrections: [{ id: "e1", count: 2 }], rules: [{ id: "r1", count: 3 }] })],
    });
    const corrections = renderHook(() => useHitTotals("corrections"), { wrapper: wrap(backend) });
    const rules = renderHook(() => useHitTotals("rules"), { wrapper: wrap(backend) });
    await waitFor(() => {
      expect([...corrections.result.current]).toEqual([["e1", 2]]);
    });
    expect([...rules.result.current]).toEqual([["r1", 3]]);
    const failing = new MockBackend();
    vi.spyOn(failing, "historyHits").mockRejectedValue(new Error("no history"));
    const none = renderHook(() => useHitTotals("rules"), { wrapper: wrap(failing) });
    await waitFor(() => {
      expect(none.result.current.size).toBe(0);
    });
    backend.destroy();
    failing.destroy();
  });
});
