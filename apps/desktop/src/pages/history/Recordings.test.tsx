// The recording cache on the history and home pages (docs/dictation.md §26), on the whole app with
// the in-memory core: 未识别的录音 with 重新识别 on chosen models, 删除录音 and 清理, the short list
// on the home page, the kept recording in an entry's detail, and 重新润色 under an entry whose
// polish failed.
import {
  type EngineSettings,
  type HistoryEntry,
  type KeptRecording,
  defaultEngineSettings,
} from "@voltip/shared";
import { MOCK_ASR_MS, MOCK_REFINE_MS, MockBackend } from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";

function entry(n: number, extra: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`,
    at_ms: Date.now() - n * 60_000,
    raw_text: `第${n}句`,
    text: `第${n}句。`,
    refined: true,
    asr_model: "Qwen/Qwen3-ASR-1.7B",
    duration_ms: 4200,
    asr_ms: 640,
    refine_ms: 380,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
    ...extra,
  };
}

function recording(
  id: string,
  status: KeptRecording["status"],
  extra: Partial<KeptRecording> = {},
): KeptRecording {
  return { id, at_ms: Date.now() - 30_000, duration_ms: 3000, bytes: 96_044, status, ...extra };
}

/** The `i`th of `items`; the test fails when there is none. */
function nth<T>(items: readonly T[], i: number): T {
  const item = items[i];
  if (item === undefined) throw new Error(`no item ${i} among ${items.length}`);
  return item;
}

/** Fake timers that still advance with the wall clock, so the page's dates and the core's
 *  recognition both follow them; `user` advances them while it types and clicks. */
function withTimers() {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  return userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
}

/** Let the in-memory core recognise and polish a take. */
async function recognise() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(MOCK_ASR_MS + MOCK_REFINE_MS);
  });
}

afterEach(() => {
  vi.useRealTimers();
});

/** The settings with a second recognition and clean-up provider set up (the custom endpoint). */
function withCustom(): EngineSettings {
  return {
    ...defaultEngineSettings(),
    providers: {
      custom: {
        asr_url: "https://asr.example.test/v1",
        asr_model: "whisper-1",
        llm_url: "https://llm.example.test/v1",
        llm_model: "gpt-4o-mini",
      },
    },
  };
}

/** Pick `model` in the menu the button `trigger` opens. */
async function pick(user: ReturnType<typeof userEvent.setup>, trigger: HTMLElement, model: string) {
  await user.click(trigger);
  const menu = screen.getByRole("menu");
  await user.click(within(menu).getByRole("menuitemradio", { name: model }));
}

describe("History · 未识别的录音", () => {
  it("lists the recordings no text came from, newest first, with why, and not the ones an entry came from", async () => {
    const recognised = entry(1);
    renderApp({
      path: "/history",
      backend: new MockBackend({
        now: () => Date.now(),
        history: [recognised],
        audioCache: [
          recording(
            "r-failed",
            { kind: "failed", code: "asr", message: "asr: 503 Service Unavailable" },
            {
              app: { id: "slack", name: "Slack" },
            },
          ),
          recording("r-cancelled", { kind: "cancelled" }),
          recording("r-interrupted", { kind: "interrupted" }),
          recording("r-linked", { kind: "recognized", entry: recognised.id }),
        ],
      }),
    });
    const card = await screen.findByTestId("history-recordings");
    expect(card).toHaveTextContent("未识别的录音");
    expect(within(card).getByTestId("history-recordings-count")).toHaveTextContent("3 条");
    const rows = within(card).getAllByTestId("history-recording");
    expect(rows.map((r) => r.dataset.recordingId)).toEqual([
      "r-failed",
      "r-cancelled",
      "r-interrupted",
    ]);
    expect(within(nth(rows, 0)).getByTestId("history-recording-status")).toHaveTextContent(
      "识别失败：识别服务出错：503 Service Unavailable",
    );
    expect(rows[0]).toHaveTextContent("在 Slack 中录制");
    expect(within(nth(rows, 1)).getByTestId("history-recording-status")).toHaveTextContent(
      "录制或识别时已取消",
    );
    expect(within(nth(rows, 2)).getByTestId("history-recording-status")).toHaveTextContent(
      "录制或处理时 Voltip 已关闭",
    );
  });

  it("shows nothing while every kept recording has its entry", async () => {
    const recognised = entry(1);
    renderApp({
      path: "/history",
      backend: new MockBackend({
        now: () => Date.now(),
        history: [recognised],
        audioCache: [recording("r-linked", { kind: "recognized", entry: recognised.id })],
      }),
    });
    await screen.findByTestId("history-detail-retake");
    expect(screen.queryByTestId("history-recordings")).toBeNull();
  });

  it("重新识别 recognises a recording again: a new entry on the clipboard, and the row is gone", async () => {
    const user = withTimers();
    const backend = new MockBackend({
      now: () => Date.now(),
      history: [entry(1)],
      audioCache: [
        recording("r-failed", { kind: "failed", code: "no_speech", message: "没有听到声音" }),
      ],
    });
    renderApp({ path: "/history", backend });
    const card = await screen.findByTestId("history-recordings");
    await user.click(within(card).getByRole("button", { name: "重新识别" }));
    // While it runs the row says so, and the other takes wait.
    expect(await within(card).findByRole("button", { name: "正在识别…" })).toBeDisabled();
    await recognise();
    expect(await screen.findByText("已重新识别，文字已复制到剪贴板")).toBeInTheDocument();
    const state = backend.peek();
    expect(state.history_recent).toHaveLength(2);
    const made = nth(state.history_recent, 0);
    // Never pasted into whatever window is in front now.
    expect(made.outcome).toEqual({ kind: "inserted", via: "clipboard" });
    expect(made.duration_ms).toBe(3000);
    expect(state.audio_cache.recordings).toEqual([
      expect.objectContaining({ id: "r-failed", status: { kind: "recognized", entry: made.id } }),
    ]);
    expect(screen.queryByTestId("history-recordings")).toBeNull();
  });

  it("a retake that fails again says why and keeps the recording", async () => {
    const user = withTimers();
    const backend = new MockBackend({
      now: () => Date.now(),
      history: [entry(1)],
      audioCache: [recording("r-cancelled", { kind: "cancelled" })],
    });
    renderApp({ path: "/history", backend });
    const card = await screen.findByTestId("history-recordings");
    await user.click(within(card).getByRole("button", { name: "重新识别" }));
    await within(card).findByRole("button", { name: "正在识别…" });
    act(() => {
      backend.simulateDictationFailed("asr: 401 Unauthorized", undefined, "asr");
    });
    expect(await screen.findByText(/^重新识别失败：识别服务出错/)).toBeInTheDocument();
    expect(backend.peek().history_recent).toHaveLength(1);
    const kept = backend.peek().audio_cache.recordings;
    expect(kept.map((r) => [r.id, r.status.kind])).toEqual([["r-cancelled", "failed"]]);
    expect(within(card).getByTestId("history-recording-status")).toHaveTextContent("识别失败：");
  });

  it("重新识别 runs on the models chosen above the list, for that take only (docs/dictation.md §26.4)", async () => {
    const user = withTimers();
    const engines = withCustom();
    const backend = new MockBackend({
      now: () => Date.now(),
      history: [entry(1)],
      settings: { engines },
      audioCache: [
        recording("r-failed", {
          kind: "failed",
          code: "asr",
          message: "asr: 503 Service Unavailable",
        }),
      ],
    });
    renderApp({ path: "/history", backend });
    const card = await screen.findByTestId("history-recordings");
    // Nothing chosen: the settings' models, as the title bar's menus show them.
    const speech = within(card).getByRole("button", {
      name: "重新识别使用的语音模型：Qwen3-ASR-1.7B",
    });
    await user.click(speech);
    const speechMenu = screen.getByRole("menu", { name: "语音模型" });
    expect(
      within(speechMenu)
        .getAllByRole("menuitemradio")
        .map((item) => [item.textContent, item.getAttribute("aria-checked")]),
    ).toEqual([
      ["Qwen3-ASR-1.7B", "true"],
      ["whisper-1", "false"],
    ]);
    await user.click(within(speechMenu).getByRole("menuitemradio", { name: "whisper-1" }));
    const polish = within(card).getByRole("button", {
      name: "重新识别使用的 AI 润色：qwen3.8-27b",
    });
    await user.click(polish);
    const polishMenu = screen.getByRole("menu", { name: "AI 润色" });
    // 不润色 first, then every configured clean-up model.
    expect(
      within(polishMenu)
        .getAllByRole("menuitemradio")
        .map((item) => item.textContent),
    ).toEqual(["不润色", "qwen3.8-27b", "gpt-oss-120b", "gpt-oss-20b", "gpt-4o-mini"]);
    await user.click(within(polishMenu).getByRole("menuitemradio", { name: "不润色" }));
    expect(
      within(card).getByRole("button", { name: "重新识别使用的 AI 润色：不润色" }),
    ).toBeInTheDocument();
    await user.click(within(card).getByRole("button", { name: "重新识别" }));
    await recognise();
    expect(await screen.findByText("已重新识别，文字已复制到剪贴板")).toBeInTheDocument();
    const made = nth(backend.peek().history_recent, 0);
    expect(made).toMatchObject({ asr_model: "whisper-1", refined: false });
    expect(made.refine_model).toBeUndefined();
    // The settings stay as they were, and so does what the next take runs on.
    expect(backend.peek().settings.engines).toEqual(engines);
    expect(backend.peek().engines.asr_model).toBe("Qwen/Qwen3-ASR-1.7B");
  });

  it("a chosen clean-up model polishes the retake with it, whatever the settings say", async () => {
    const user = withTimers();
    const backend = new MockBackend({
      now: () => Date.now(),
      history: [entry(1)],
      settings: { engines: { ...withCustom(), refine_enabled: false } },
      audioCache: [recording("r1", { kind: "cancelled" })],
    });
    renderApp({ path: "/history", backend });
    const card = await screen.findByTestId("history-recordings");
    // The settings polish nothing: 不润色 is what the menu shows.
    await pick(
      user,
      within(card).getByRole("button", { name: "重新识别使用的 AI 润色：不润色" }),
      "gpt-4o-mini",
    );
    await user.click(within(card).getByRole("button", { name: "重新识别" }));
    await recognise();
    expect(await screen.findByText("已重新识别，文字已复制到剪贴板")).toBeInTheDocument();
    expect(nth(backend.peek().history_recent, 0)).toMatchObject({
      refined: true,
      refine_model: "gpt-4o-mini",
      asr_model: "Qwen/Qwen3-ASR-1.7B",
    });
    expect(backend.peek().settings.engines.refine_enabled).toBe(false);
  });

  it("清理 deletes every recording no text came from after asking, and keeps the ones an entry came from", async () => {
    const user = userEvent.setup();
    const recognised = entry(1);
    const backend = new MockBackend({
      now: () => Date.now(),
      history: [recognised],
      audioCache: [
        recording("r1", { kind: "cancelled" }),
        recording("r2", { kind: "interrupted" }),
        recording("r-linked", { kind: "recognized", entry: recognised.id }),
      ],
    });
    renderApp({ path: "/history", backend });
    const card = await screen.findByTestId("history-recordings");
    await user.click(within(card).getByRole("button", { name: "清理" }));
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent("清理未识别的录音？");
    expect(dialog).toHaveTextContent("2 条未识别的录音会被删除，删除后不能再重新识别。");
    // Cancelled: nothing goes.
    await user.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(backend.peek().audio_cache.recordings).toHaveLength(3);
    await user.click(within(card).getByRole("button", { name: "清理" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "清理" }));
    await waitFor(() => {
      expect(backend.peek().audio_cache.recordings.map((r) => r.id)).toEqual(["r-linked"]);
    });
    expect(await screen.findByText("已清理 2 条录音")).toBeInTheDocument();
    expect(screen.queryByTestId("history-recordings")).toBeNull();
  });

  it("the list keeps to about three rows and scrolls the rest, so the log and the detail keep the window", async () => {
    renderApp({
      path: "/history",
      backend: new MockBackend({
        now: () => Date.now(),
        history: [entry(1)],
        audioCache: Array.from({ length: 6 }, (_, i) =>
          recording(`r${String(i)}`, { kind: "cancelled" }),
        ),
      }),
    });
    const list = await screen.findByTestId("history-recordings-list");
    expect(within(list).getAllByTestId("history-recording")).toHaveLength(6);
    expect(list).toHaveClass("max-h-[176px]", "overflow-y-auto");
  });

  it("删除录音 deletes that recording only", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({
      now: () => Date.now(),
      history: [entry(1)],
      audioCache: [
        recording("r1", { kind: "cancelled" }),
        recording("r2", { kind: "interrupted" }),
      ],
    });
    renderApp({ path: "/history", backend });
    const card = await screen.findByTestId("history-recordings");
    const first = nth(within(card).getAllByTestId("history-recording"), 0);
    await user.click(within(first).getByRole("button", { name: "删除录音" }));
    await waitFor(() => {
      expect(backend.peek().audio_cache.recordings.map((r) => r.id)).toEqual(["r2"]);
    });
    expect(await screen.findByText("已删除录音")).toBeInTheDocument();
  });

  it("deleting an entry deletes the recording it came from", async () => {
    const first = entry(1);
    const backend = new MockBackend({
      now: () => Date.now(),
      history: [first, entry(2)],
      audioCache: [recording("r-linked", { kind: "recognized", entry: first.id })],
    });
    renderApp({ path: "/history", backend });
    await screen.findByTestId("history-detail-retake");
    await act(async () => {
      await backend.invoke("history_delete", { id: first.id });
    });
    expect(backend.peek().audio_cache.recordings).toEqual([]);
    await waitFor(() => {
      expect(screen.queryByTestId("history-kept-recording")).toBeNull();
    });
  });
});

describe("History · an entry's kept recording and 重新润色", () => {
  it("an entry whose recording is kept offers 重新识别, which makes a new entry from it", async () => {
    const user = withTimers();
    const first = entry(1);
    const backend = new MockBackend({
      now: () => Date.now(),
      history: [first],
      audioCache: [recording("r-linked", { kind: "recognized", entry: first.id })],
    });
    renderApp({ path: "/history", backend });
    const kept = await screen.findByTestId("history-kept-recording");
    expect(kept).toHaveTextContent("这条听写的录音仍保留在本机");
    expect(kept).toHaveTextContent("结果作为一条新记录保存，并复制到剪贴板");
    // The same choice of models as the list's, here too.
    expect(within(kept).getByTestId("retake-models")).toHaveTextContent("重新识别使用");
    await user.click(within(kept).getByRole("button", { name: "重新识别" }));
    await recognise();
    expect(await screen.findByText("已重新识别，文字已复制到剪贴板")).toBeInTheDocument();
    const state = backend.peek();
    expect(state.history_recent.map((e) => e.id)).toContain(first.id);
    expect(state.history_recent).toHaveLength(2);
    // The recording now belongs to the new entry.
    expect(state.audio_cache.recordings[0]?.status).toEqual({
      kind: "recognized",
      entry: state.history_recent[0]?.id,
    });
  });

  it("an entry's 重新识别 can run on another speech model, for that take only", async () => {
    const user = withTimers();
    const first = entry(1);
    const engines = withCustom();
    const backend = new MockBackend({
      now: () => Date.now(),
      history: [first],
      settings: { engines },
      audioCache: [recording("r-linked", { kind: "recognized", entry: first.id })],
    });
    renderApp({ path: "/history", backend });
    const kept = await screen.findByTestId("history-kept-recording");
    await pick(
      user,
      within(kept).getByRole("button", { name: "重新识别使用的语音模型：Qwen3-ASR-1.7B" }),
      "whisper-1",
    );
    await user.click(within(kept).getByRole("button", { name: "重新识别" }));
    await recognise();
    expect(await screen.findByText("已重新识别，文字已复制到剪贴板")).toBeInTheDocument();
    expect(nth(backend.peek().history_recent, 0).asr_model).toBe("whisper-1");
    expect(backend.peek().settings.engines).toEqual(engines);
  });

  it("an entry without a kept recording has no 重新识别", async () => {
    renderApp({
      path: "/history",
      backend: new MockBackend({ now: () => Date.now(), history: [entry(1)] }),
    });
    await screen.findByTestId("entry-text");
    expect(screen.queryByTestId("history-kept-recording")).toBeNull();
  });

  it("重新润色 polishes a take whose polish failed and shows the result as 处理后", async () => {
    const user = withTimers();
    const failed = entry(1, {
      refined: false,
      refine_ms: undefined,
      refine_failure: "failed",
      refine_error: "503 Service Unavailable",
    });
    const backend = new MockBackend({ now: () => Date.now(), history: [failed] });
    renderApp({ path: "/history", backend });
    const note = await screen.findByTestId("history-refine-note");
    await user.click(within(note).getByRole("button", { name: "重新润色" }));
    expect(await within(note).findByRole("button", { name: "正在润色…" })).toBeDisabled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(MOCK_REFINE_MS);
    });
    expect(await screen.findByText("已重新润色，可在「处理后」查看")).toBeInTheDocument();
    const processed = backend.peek().history_recent[0]?.processed;
    expect(processed?.preset).toEqual({ id: "proofread", name: "校对" });
    expect(screen.getByRole("radio", { name: "处理后" })).toBeChecked();
    expect(screen.getByTestId("entry-processed")).toHaveTextContent(processed?.text ?? "none");
    // The entry's own text stays what was inserted.
    expect(backend.peek().history_recent[0]?.text).toBe(failed.text);
  });

  it("without an AI service there is no 重新润色", async () => {
    renderApp({
      path: "/history",
      backend: new MockBackend({
        now: () => Date.now(),
        history: [
          entry(1, {
            refined: false,
            refine_failure: "failed",
            refine_error: "503 Service Unavailable",
          }),
        ],
        engines: { refine_ready: false },
      }),
    });
    const note = await screen.findByTestId("history-refine-note");
    expect(within(note).queryByRole("button", { name: "重新润色" })).toBeNull();
  });
});

describe("Home · 未识别的录音 (user request 2026-10-26)", () => {
  it("shows the newest three above 最近的结果, one line each, with 查看全部 for the rest", async () => {
    const user = userEvent.setup();
    const recognised = entry(1);
    const backend = new MockBackend({
      now: () => Date.now(),
      history: [recognised],
      audioCache: [
        recording(
          "r-failed",
          { kind: "failed", code: "asr", message: "asr: 503 Service Unavailable" },
          { app: { id: "slack", name: "Slack" } },
        ),
        recording("r-cancelled", { kind: "cancelled" }),
        recording("r-interrupted", { kind: "interrupted" }),
        recording("r-older", { kind: "cancelled" }, { at_ms: Date.now() - 3_600_000 }),
        recording("r-linked", { kind: "recognized", entry: recognised.id }),
      ],
    });
    renderApp({ path: "/", backend });
    const card = await screen.findByTestId("home-recordings");
    // Above the recent results.
    const recent = screen.getByTestId("home-recent");
    expect(card.compareDocumentPosition(recent) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(card).getByTestId("home-recordings-count")).toHaveTextContent("4 条");
    const rows = within(card).getAllByTestId("home-recording");
    expect(rows.map((r) => r.dataset.recordingId)).toEqual([
      "r-failed",
      "r-cancelled",
      "r-interrupted",
    ]);
    expect(nth(rows, 0)).toHaveTextContent("在 Slack 中录制");
    expect(within(nth(rows, 0)).getByTestId("home-recording-status")).toHaveTextContent(
      "识别失败：识别服务出错：503 Service Unavailable",
    );
    // The whole line on hover, where a narrow window cuts it.
    expect(within(nth(rows, 1)).getByTitle(/录制或识别时已取消/)).toBeInTheDocument();
    await user.click(within(card).getByRole("button", { name: "查看全部 4 条" }));
    expect(await screen.findByTestId("page-history")).toBeInTheDocument();
    expect(screen.getAllByTestId("history-recording")).toHaveLength(4);
  });

  it("shows nothing, and no 查看全部, while there is nothing to recognise again", async () => {
    const recognised = entry(1);
    renderApp({
      path: "/",
      backend: new MockBackend({
        now: () => Date.now(),
        history: [recognised],
        audioCache: [
          recording("r-linked", { kind: "recognized", entry: recognised.id }),
          recording("r-pending", { kind: "pending" }),
        ],
      }),
    });
    await screen.findByTestId("home-recent");
    expect(screen.queryByTestId("home-recordings")).toBeNull();
  });

  it("重新识别 runs on the chosen models right there, and 清理 empties the list after asking", async () => {
    const user = withTimers();
    const engines = withCustom();
    const backend = new MockBackend({
      now: () => Date.now(),
      history: [entry(1)],
      settings: { engines },
      audioCache: [
        recording("r1", { kind: "cancelled" }),
        recording("r2", { kind: "interrupted" }),
      ],
    });
    renderApp({ path: "/", backend });
    const card = await screen.findByTestId("home-recordings");
    expect(within(card).queryByRole("button", { name: /^查看全部/ })).toBeNull();
    await pick(
      user,
      within(card).getByRole("button", { name: "重新识别使用的语音模型：Qwen3-ASR-1.7B" }),
      "whisper-1",
    );
    const first = nth(within(card).getAllByTestId("home-recording"), 0);
    await user.click(within(first).getByRole("button", { name: "重新识别" }));
    // While it runs the row says so and the other one waits.
    expect(await within(first).findByRole("button", { name: "正在识别…" })).toBeDisabled();
    const second = nth(within(card).getAllByTestId("home-recording"), 1);
    expect(within(second).getByRole("button", { name: "重新识别" })).toBeDisabled();
    await recognise();
    expect(await screen.findByText("已重新识别，文字已复制到剪贴板")).toBeInTheDocument();
    expect(nth(backend.peek().history_recent, 0).asr_model).toBe("whisper-1");
    expect(backend.peek().settings.engines).toEqual(engines);
    expect(
      within(card)
        .getAllByTestId("home-recording")
        .map((r) => r.dataset.recordingId),
    ).toEqual(["r2"]);
    await user.click(within(card).getByRole("button", { name: "清理" }));
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent("1 条未识别的录音会被删除，删除后不能再重新识别。");
    await user.click(within(dialog).getByRole("button", { name: "清理" }));
    await waitFor(() => {
      expect(screen.queryByTestId("home-recordings")).toBeNull();
    });
    // The recognised one stays with the entry made from it.
    expect(backend.peek().audio_cache.recordings.map((r) => r.id)).toEqual(["r1"]);
  });
});
