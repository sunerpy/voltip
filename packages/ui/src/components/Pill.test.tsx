import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  LIVE_CAPTION_MAX_CHARS,
  LiveCaption,
  PILL_CAPTIONS,
  PILL_DEFAULT_LABEL,
  PILL_STATES,
  Pill,
  clipLiveCaption,
  clipTail,
} from "./Pill";

describe("Pill", () => {
  it("renders all eight states plus blocked, unfocusable", () => {
    for (const state of PILL_STATES) {
      const { unmount } = render(<Pill state={state} levels={[0.2, 0.8]} />);
      const pill = screen.getByRole("status");
      expect(pill).toHaveAttribute("data-state", state);
      expect(pill).toHaveAttribute("tabindex", "-1");
      expect(PILL_CAPTIONS[state].length).toBeGreaterThan(0);
      unmount();
    }
  });

  it("shows the default label for every state that carries text", () => {
    // armed is the compact three-dot capsule: its label is the accessible name, not drawn text.
    const labelled = PILL_STATES.filter(
      (s) => s !== "listening" && s !== "locked" && s !== "armed",
    );
    for (const state of labelled) {
      const { unmount } = render(<Pill state={state} />);
      expect(screen.getByText(PILL_DEFAULT_LABEL[state])).toBeInTheDocument();
      unmount();
    }
    render(<Pill state="armed" />);
    expect(
      screen.getByRole("img", { name: new RegExp(PILL_DEFAULT_LABEL.armed) }),
    ).toBeInTheDocument();
  });

  it("wires copy / stop actions and optional readouts", async () => {
    const user = userEvent.setup();
    const onCopy = vi.fn();
    const onStop = vi.fn();
    render(<Pill state="error" label="未插入 · 目标窗口没有焦点" onCopy={onCopy} />);
    await user.click(screen.getByRole("button", { name: "复制文本" }));
    expect(onCopy).toHaveBeenCalled();
    render(<Pill state="locked" readout="01:24" onStop={onStop} mode="云端 openai" />);
    await user.click(screen.getByRole("button", { name: "结束录音" }));
    expect(onStop).toHaveBeenCalled();
    expect(screen.getByText("01:24")).toBeInTheDocument();
    expect(screen.getByText("云端 openai")).toBeInTheDocument();
    render(
      <Pill state="inserted" label="已插入 · 42 字" via="VS Code" readout="0.8 s · WM_PASTE" />,
    );
    expect(screen.getByText("→ VS Code")).toBeInTheDocument();
    render(<Pill state="cancel-armed" readout="Esc 已按下 · 00:07" />);
    expect(screen.getByText("Esc 已按下 · 00:07")).toBeInTheDocument();
    render(<Pill state="listening" readout="00:07" keys="Esc" />);
    expect(screen.getByText("00:07")).toBeInTheDocument();
    render(<Pill state="processing" readout="0.9 s" label="润色中" mode="LLM" />);
    expect(screen.getByText("润色中")).toBeInTheDocument();
    render(<Pill state="armed" keys="Ctrl Alt Space" />);
    // Compact resting capsule: the hotkey is announced, not drawn (52 px pill).
    expect(screen.getByRole("img", { name: /Ctrl Alt Space/ })).toBeInTheDocument();
  });
});

describe("Pill cancel hint and step time (user feedback 2026-09-29)", () => {
  it("regression: the Esc hint reads as cancel in the danger colour", () => {
    // A running take cancels on Esc; the bare keycap did not say so.
    for (const state of ["listening", "locked", "processing"] as const) {
      const { unmount } = render(<Pill state={state} />);
      const hint = screen.getByRole("img", { name: "按 Esc 取消这次录音" });
      expect(hint).toHaveTextContent("Esc取消");
      expect(hint).toHaveClass("text-danger");
      expect(hint.querySelector("kbd")).toHaveClass("border-danger", "text-danger");
      unmount();
    }
    for (const state of ["armed", "inserted", "error", "cancel-armed", "blocked"] as const) {
      const { unmount } = render(<Pill state={state} />);
      expect(screen.queryByTestId("pill-esc-cancel")).toBeNull();
      unmount();
    }
  });

  it("regression: a processing pill shows the step time it is given and no fixed 0.0 s", () => {
    const { rerender } = render(<Pill state="processing" />);
    expect(screen.queryByTestId("pill-stage-time")).toBeNull();
    expect(screen.getByRole("status")).not.toHaveTextContent("0.0 s");
    rerender(<Pill state="processing" readout="1.4 s" />);
    expect(screen.getByTestId("pill-stage-time")).toHaveTextContent("1.4 s");
  });
});

describe("Pill live preview (docs/dictation.md §11)", () => {
  it("regression: listening draws a two-tone live caption above the waveform with a 预览 chip; the capsule grows to two rows", () => {
    const { rerender } = render(
      <Pill
        state="listening"
        readout="00:03"
        live={{ committed: "把 fetchUser 改成 async，", current: "然后加上错误" }}
      />,
    );
    const pill = screen.getByRole("status");
    expect(pill).toHaveClass("h-14");
    expect(pill).not.toHaveClass("h-10");
    const line = screen.getByTestId("pill-live");
    expect(screen.getByTestId("pill-live-committed")).toHaveTextContent(
      "把 fetchUser 改成 async，",
    );
    expect(screen.getByTestId("pill-live-committed")).not.toHaveClass("text-pill-muted");
    expect(screen.getByTestId("pill-live-current")).toHaveTextContent("然后加上错误");
    expect(screen.getByTestId("pill-live-current")).toHaveClass("text-pill-muted");
    // CJK boundary: no space between the committed sentence and the current one.
    expect(line).toHaveTextContent(/把 fetchUser 改成 async，然后加上错误\s*预览$/);
    expect(line).not.toHaveAttribute("data-clipped");
    // The waveform row is still there beneath, with the timer counting.
    expect(pill).toHaveTextContent("00:03");
    expect(pill).toHaveTextContent("本地");
    // Latin boundary: one space between committed and current.
    rerender(<Pill state="listening" live={{ committed: "Hello.", current: "How are" }} />);
    expect(screen.getByTestId("pill-live")).toHaveTextContent(/^Hello\. How are/);
    // Only the current sentence so far: nothing committed yet.
    rerender(<Pill state="listening" live={{ committed: "", current: "把这段" }} />);
    expect(screen.getByTestId("pill-live-committed")).toBeEmptyDOMElement();
    expect(screen.getByTestId("pill-live-current")).toHaveTextContent("把这段");
    // Without a preview the capsule is the plain 40 px listening pill.
    rerender(<Pill state="listening" readout="00:03" />);
    expect(screen.getByRole("status")).toHaveClass("h-10");
    expect(screen.queryByTestId("pill-live")).toBeNull();
  });

  it("regression: a long live caption keeps its tail — a leading ellipsis, at most 40 characters, the current sentence always visible", () => {
    const committed = "一二三四五六七八九十".repeat(4); // 40 chars
    render(<Pill state="listening" live={{ committed, current: "正在说的这句" }} />);
    const line = screen.getByTestId("pill-live");
    expect(line).toHaveAttribute("data-clipped", "true");
    const shown = screen.getByTestId("pill-live-committed").textContent ?? "";
    expect(shown.startsWith("…")).toBe(true);
    expect(Array.from(shown.slice(1)).length + Array.from("正在说的这句").length).toBe(
      LIVE_CAPTION_MAX_CHARS,
    );
    expect(screen.getByTestId("pill-live-current")).toHaveTextContent("正在说的这句");
    // Pure helpers: committed is cut first; a current sentence longer than the budget is cut too.
    expect(clipLiveCaption({ committed: "abc", current: "de" }, 5)).toEqual({
      committed: "abc",
      current: "de",
      clipped: false,
    });
    expect(clipLiveCaption({ committed: "abcdef", current: "gh" }, 5)).toEqual({
      committed: "…def",
      current: "gh",
      clipped: true,
    });
    expect(clipLiveCaption({ committed: "ab", current: "cdefgh" }, 4)).toEqual({
      committed: "",
      current: "…efgh",
      clipped: true,
    });
    // Exactly the committed text is cut: the ellipsis still marks the cut.
    expect(clipLiveCaption({ committed: "abc", current: "defgh" }, 5)).toEqual({
      committed: "",
      current: "…defgh",
      clipped: true,
    });
    expect(clipTail("把 fetchUser 改成 async，然后加上错误处理。", 6)).toBe("…上错误处理。");
    expect(clipTail("short")).toBe("short");
  });

  it("regression: a locked take shows the lock mark in place of the lamp and keeps the caption; pasted sentences are drawn fainter and are the first to be clipped (docs/dictation.md §12–§13)", () => {
    const { rerender } = render(<Pill state="listening" readout="00:09" locked />);
    const lock = screen.getByTestId("pill-lock");
    expect(lock).toHaveAttribute("aria-label", "已锁定 · 再按一次结束");
    expect(lock.querySelector('[data-icon="lock"]')).not.toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent("00:09");
    expect(screen.getByRole("status")).toHaveAttribute("data-state", "listening");
    rerender(<Pill state="listening" readout="00:09" />);
    expect(screen.queryByTestId("pill-lock")).toBeNull();
    // live_inject: the pasted sentence is fainter than the committed one, which is fainter than
    // nothing; the current sentence keeps its muted tone; CJK boundaries get no space.
    rerender(
      <Pill
        state="listening"
        locked
        live={{ injected: "把这段逻辑抽成一个 helper，", committed: "然后复用。", current: "最后" }}
      />,
    );
    expect(screen.getByTestId("pill-lock")).toBeInTheDocument();
    const injected = screen.getByTestId("pill-live-injected");
    expect(injected).toHaveTextContent("把这段逻辑抽成一个 helper，");
    expect(injected).toHaveClass("text-pill-muted", "opacity-60");
    expect(injected).toHaveAttribute("title", "已输入到当前窗口");
    expect(screen.getByTestId("pill-live-committed")).toHaveTextContent("然后复用。");
    expect(screen.getByTestId("pill-live-committed")).not.toHaveClass("text-pill-muted");
    expect(screen.getByTestId("pill-live")).toHaveTextContent(
      /^把这段逻辑抽成一个 helper，然后复用。最后\s*预览$/,
    );
    // Nothing pasted yet: no injected span at all.
    rerender(<Pill state="listening" live={{ injected: "", committed: "a.", current: "b" }} />);
    expect(screen.queryByTestId("pill-live-injected")).toBeNull();
    expect(screen.getByTestId("pill-live")).toHaveTextContent(/^a\. b/);
    // Clipping drops the pasted text first, then the committed text; the ellipsis marks the first
    // surviving part; the `injected` key is only reported when it was given.
    expect(clipLiveCaption({ injected: "abc", committed: "def", current: "gh" }, 6)).toEqual({
      injected: "…c",
      committed: "def",
      current: "gh",
      clipped: true,
    });
    expect(clipLiveCaption({ injected: "abc", committed: "def", current: "gh" }, 4)).toEqual({
      injected: "",
      committed: "…ef",
      current: "gh",
      clipped: true,
    });
    expect(clipLiveCaption({ injected: "abc", committed: "de", current: "fgh" }, 2)).toEqual({
      injected: "",
      committed: "",
      current: "…gh",
      clipped: true,
    });
    expect(clipLiveCaption({ injected: "ab", committed: "c", current: "d" }, 9)).toEqual({
      injected: "ab",
      committed: "c",
      current: "d",
      clipped: false,
    });
    expect(clipLiveCaption({ committed: "abcdef", current: "gh" }, 5)).not.toHaveProperty(
      "injected",
    );
  });

  it("regression: waiting for the microphone parks the timer at 00:00 with a hint; processing shows the preview in place of the stage caption", () => {
    const { rerender } = render(<Pill state="listening" readout="00:07" waiting />);
    expect(screen.getByRole("status")).toHaveTextContent("00:00");
    expect(screen.getByRole("status")).not.toHaveTextContent("00:07");
    expect(screen.getByTestId("pill-waiting")).toHaveTextContent("等待麦克风");
    rerender(<Pill state="listening" readout="00:07" />);
    expect(screen.queryByTestId("pill-waiting")).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent("00:07");
    rerender(
      <Pill state="processing" label="识别中…" preview="把 fetchUser 改成 async，然后加上错误" />,
    );
    expect(screen.getByTestId("pill-preview")).toHaveTextContent(
      "把 fetchUser 改成 async，然后加上错误",
    );
    expect(screen.getByTestId("pill-preview")).toHaveClass("text-pill-muted");
    expect(screen.queryByText("识别中…")).toBeNull();
    // An empty preview falls back to the caption; the final result never shows a preview.
    rerender(<Pill state="processing" label="识别中…" preview="" />);
    expect(screen.getByText("识别中…")).toBeInTheDocument();
    expect(screen.queryByTestId("pill-preview")).toBeNull();
    rerender(<Pill state="inserted" label="已插入 3 字" preview="ignored" />);
    expect(screen.queryByTestId("pill-preview")).toBeNull();
  });

  it("regression: the matched scene is a tag right after the mode tag while listening, locked and processing, and nowhere else (docs/dictation.md section 18.6)", () => {
    const { rerender } = render(<Pill state="listening" mode="云端" scene="聊天" />);
    const tag = screen.getByTestId("pill-scene");
    expect(tag).toHaveTextContent("聊天");
    expect(tag).toHaveAttribute("title", "场景：聊天");
    // Right after the mode tag.
    expect(screen.getByText("云端").nextElementSibling).toBe(tag);
    rerender(<Pill state="locked" mode="云端" scene="聊天" />);
    expect(screen.getByText("云端").nextElementSibling).toBe(screen.getByTestId("pill-scene"));
    rerender(<Pill state="processing" mode="LLM" scene="聊天" />);
    expect(screen.getByText("LLM").nextElementSibling).toBe(screen.getByTestId("pill-scene"));
    for (const state of ["inserted", "error", "armed"] as const) {
      rerender(<Pill state={state} scene="聊天" />);
      expect(screen.queryByTestId("pill-scene")).toBeNull();
    }
    // No scene, or an empty name: no tag.
    rerender(<Pill state="listening" mode="云端" />);
    expect(screen.queryByTestId("pill-scene")).toBeNull();
    rerender(<Pill state="listening" mode="云端" scene="" />);
    expect(screen.queryByTestId("pill-scene")).toBeNull();
  });

  it("regression: a voice edit leads the capsule with its kind tag in every state of a take and the resting and blocked capsules never show it (section 19)", () => {
    const { container, rerender } = render(<Pill state="listening" mode="云端" tag="编辑" />);
    const tag = screen.getByTestId("pill-tag");
    expect(tag).toHaveTextContent("编辑");
    expect(container.querySelector('[role="status"]')?.firstElementChild).toBe(tag);
    // With the live caption the capsule has two rows; the tag still leads.
    rerender(<Pill state="listening" tag="编辑" live={{ committed: "", current: "改得更正式" }} />);
    expect(container.querySelector('[role="status"]')?.firstElementChild).toBe(
      screen.getByTestId("pill-tag"),
    );
    for (const state of ["locked", "processing", "inserted", "error", "cancel-armed"] as const) {
      rerender(<Pill state={state} tag="编辑" />);
      expect(screen.getByTestId("pill-tag")).toHaveTextContent("编辑");
    }
    for (const state of ["armed", "blocked"] as const) {
      rerender(<Pill state={state} tag="编辑" />);
      expect(screen.queryByTestId("pill-tag")).toBeNull();
    }
    // A dictation carries no tag.
    rerender(<Pill state="listening" />);
    expect(screen.queryByTestId("pill-tag")).toBeNull();
    rerender(<Pill state="listening" tag="" />);
    expect(screen.queryByTestId("pill-tag")).toBeNull();
  });
});

describe("LiveCaption", () => {
  it("shows preview / final / failed tiers, ellipsis when empty and queue depth", () => {
    const { rerender } = render(
      <LiveCaption committed="" tail="" tier="preview" elapsed="00:00" engine="zipformer · 本地" />,
    );
    expect(screen.getByText("……")).toBeInTheDocument();
    expect(screen.getByText("预览")).toBeInTheDocument();
    rerender(
      <LiveCaption
        committed="把 fetchUser 改成 async"
        tail="然后加三次 retry"
        tier="final"
        elapsed="00:12"
        engine="e"
        queueDepth={1}
      />,
    );
    expect(screen.getByText("最终")).toHaveClass("bg-accent-soft");
    expect(screen.getByText("然后加三次 retry")).toHaveClass("text-fg-subtle");
    expect(screen.getByText("1 条排队中")).toBeInTheDocument();
    rerender(<LiveCaption committed="x" tail="" tier="failed" elapsed="00:01" engine="e" />);
    expect(screen.getByText("出错了")).toBeInTheDocument();
    expect(screen.getByText("x")).toHaveClass("text-fg-subtle");
  });
});

describe("Pill width", () => {
  it("regression: the widest pill stays inside the 480 px overlay window; the waveform gives way", () => {
    // Browser measurement 2026-09-29: a long scene name, the waiting hint and the Esc hint made the
    // listening pill 533 px wide, and the 480 px overlay window cut it off.
    for (const state of ["listening", "locked", "processing"] as const) {
      const { container, unmount } = render(
        <Pill
          state={state}
          waiting={state === "listening"}
          scene="代码评审与重构场景名称很长"
          mode="云端"
        />,
      );
      const pill = container.querySelector('[role="status"]');
      expect(pill?.className, `${state} pill`).toContain("max-w-[464px]");
      const wave = container.querySelector('[role="img"][data-state]');
      for (const cls of ["min-w-0", "shrink", "justify-end", "overflow-hidden"]) {
        expect(wave?.className, `${state} waveform`).toContain(cls);
      }
      unmount();
    }
  });
});
