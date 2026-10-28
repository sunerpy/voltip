// The phone's talk card (docs/dictation.md §20.11): holding or tapping (user request 2026-10-13),
// how far a held finger may slip before letting go cancels, and how a take the computer recognised
// but could not insert reads.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { type DictationPhase, type PhoneTakeState, idleDictation, zhT } from "@voltip/shared";
import type { MockBackend } from "@voltip/shared/mock";

import { openTab, renderApp } from "../test/render";
import { CANCEL_DISTANCE, levelFraction, localTakeLine, phoneTakeLine } from "./PhoneMic";

/** A press at a point on the page; `at` is where the finger is over the child it touches. */
function touch(page: { x: number; y: number }, at = { x: 22, y: 22 }) {
  return { nativeEvent: { pageX: page.x, pageY: page.y, locationX: at.x, locationY: at.y } };
}

describe("the talk card", () => {
  it("regression: a finger that slips while holding still sends; only a release well away cancels", async () => {
    // User report 2026-10-13: letting go after a small slip cancelled the take. The check read
    // `locationX`, which is relative to the icon under the finger, so a slip to the left or up
    // inside the button already counted as leaving it.
    const { backend } = await renderApp();
    const invoke = jest.spyOn(backend, "invoke");
    const button = await screen.findByTestId("phone-mic-hold");
    await fireEvent(button, "responderGrant", touch({ x: 200, y: 500 }));
    // 50 dp to the left and 30 up, still on the button: over the 44 dp icon the finger pressed in
    // the middle of, this reads x = -28, past the old margin of 24.
    await fireEvent(button, "responderMove", touch({ x: 150, y: 500 }, { x: -28, y: 22 }));
    expect(screen.getByTestId("phone-mic-label")).toHaveTextContent("松开识别");
    await fireEvent(button, "responderRelease", touch({ x: 150, y: 470 }, { x: -28, y: -8 }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictation_stop");
    });
    expect(invoke).not.toHaveBeenCalledWith("dictation_cancel");
    await waitFor(() => {
      expect(screen.getByTestId("phone-mic-label")).toHaveTextContent("按住说话");
    });

    // A release further than CANCEL_DISTANCE from the press cancels.
    await waitFor(() => {
      expect(screen.getByTestId("phone-mic-hold")).toBeEnabled();
    });
    invoke.mockClear();
    await fireEvent(button, "responderGrant", touch({ x: 200, y: 500 }));
    const away = { x: 200, y: 500 - CANCEL_DISTANCE - 4 };
    await fireEvent(button, "responderMove", touch(away));
    expect(await screen.findByText("松开取消")).toBeOnTheScreen();
    await fireEvent(button, "responderRelease", touch(away));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictation_cancel");
    });
    expect(invoke).not.toHaveBeenCalledWith("dictation_stop");
  });

  it("regression: a held button keeps its gesture from the page's scroll view", async () => {
    // React Native hands the gesture to a native responder (the page's scroll view, once the page
    // can scroll) unless the grant returns true; a finger that moved up or down while holding then
    // scrolled the page and the take was cancelled.
    await renderApp();
    const button = await screen.findByTestId("phone-mic-hold");
    let blocks: unknown;
    await act(async () => {
      blocks = (button.props as { onResponderGrant: (e: unknown) => unknown }).onResponderGrant(
        touch({ x: 200, y: 500 }),
      );
    });
    expect(blocks).toBe(true);
    expect(screen.getByTestId("phone-mic-label")).toHaveTextContent("松开识别");
    await fireEvent(button, "responderRelease", touch({ x: 200, y: 500 }));
  });

  it("点按说话 under the button: a tap starts, the next tap sends, 取消录音 discards", async () => {
    const { backend } = await renderApp();
    const invoke = jest.spyOn(backend, "invoke");
    expect(await screen.findByTestId("phone-talk-mode-hold")).toBeChecked();
    expect(screen.getByText("手指滑出按钮再松开则取消")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("phone-talk-mode-tap"));
    await waitFor(() => {
      expect(backend.peek().settings.phone_talk).toBe("tap");
    });
    expect(screen.getByTestId("phone-talk-mode-tap")).toBeChecked();
    expect(screen.getByTestId("phone-mic-label")).toHaveTextContent("点按开始说话");
    expect(screen.getByText("点按开始录音，再次点按结束")).toBeOnTheScreen();

    // The first tap starts; letting go does not stop it.
    const button = screen.getByTestId("phone-mic-hold");
    await fireEvent(button, "responderGrant", touch({ x: 200, y: 500 }));
    await fireEvent(button, "responderRelease", touch({ x: 200, y: 500 }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictation_start");
    });
    expect(invoke).not.toHaveBeenCalledWith("dictation_stop");
    expect(screen.getByTestId("phone-mic-label")).toHaveTextContent("再次点按，开始识别");
    // The mode cannot change under a running take.
    expect(screen.getByTestId("phone-talk-mode-hold")).toBeDisabled();
    // The next tap sends.
    await fireEvent(button, "responderGrant", touch({ x: 200, y: 500 }));
    await fireEvent(button, "responderRelease", touch({ x: 200, y: 500 }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictation_stop");
    });
    await waitFor(() => {
      expect(screen.getByTestId("phone-mic-label")).toHaveTextContent("点按开始说话");
    });

    // A take tapped open is discarded with 取消录音.
    await waitFor(() => {
      expect(screen.getByTestId("phone-talk-mode-hold")).toBeEnabled();
    });
    invoke.mockClear();
    await fireEvent(button, "responderGrant", touch({ x: 200, y: 500 }));
    await fireEvent(button, "responderRelease", touch({ x: 200, y: 500 }));
    await fireEvent.press(await screen.findByTestId("phone-mic-cancel"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictation_cancel");
    });
    expect(invoke).not.toHaveBeenCalledWith("dictation_stop");
    expect(screen.queryByTestId("phone-mic-cancel")).toBeNull();

    // The choice stays with the phone's settings.
    await openTab("settings");
    await openTab("talk");
    expect(await screen.findByTestId("phone-talk-mode-tap")).toBeChecked();
  });

  it("点按说话: after a start the phone refused, the next tap starts again", async () => {
    const { backend } = await renderApp();
    const invoke = jest.spyOn(backend, "invoke");
    await fireEvent.press(await screen.findByTestId("phone-talk-mode-tap"));
    await waitFor(() => {
      expect(backend.peek().settings.phone_talk).toBe("tap");
    });
    invoke.mockClear();
    invoke.mockRejectedValueOnce(new Error("MICROPHONE_DENIED"));
    const button = screen.getByTestId("phone-mic-hold");
    await fireEvent(button, "responderGrant", touch({ x: 200, y: 500 }));
    await waitFor(() => {
      expect(screen.getByTestId("phone-mic-label")).toHaveTextContent("点按开始说话");
    });
    await fireEvent(button, "responderGrant", touch({ x: 200, y: 500 }));
    await waitFor(() => {
      expect(invoke.mock.calls.filter(([name]) => name === "dictation_start")).toHaveLength(2);
    });
    expect(screen.getByTestId("phone-mic-label")).toHaveTextContent("再次点按，开始识别");
    expect(invoke).not.toHaveBeenCalledWith("dictation_stop");
  });

  it("regression: a take the computer recognised but could not insert is a notice with its text", () => {
    // User report 2026-10-13: 「未插入：inject: keystroke: enigo: input refused: …」 although the
    // computer had the text in its history.
    const t = zhT.t;
    expect(
      phoneTakeLine(
        {
          state: "failed",
          code: "failed",
          message: "inject: keystroke: enigo: input refused: blocked by UIPI",
          text: "把 fetchUser 改成 async",
        },
        0,
        t,
      ),
    ).toBe("未能插入电脑当前的窗口，文字已保存在电脑的历史记录中：把 fetchUser 改成 async");
    // A failure with nothing recognised still says what went wrong.
    expect(phoneTakeLine({ state: "failed", code: "failed", message: "asr: 401" }, 0, t)).toBe(
      "未插入：asr: 401",
    );
  });
});

const KEY = "1".repeat(64);
const STUDIO = {
  device: {
    device_id: "0f3f1a1e-8d4b-4c8e-9f7a-1c2d3e4f5a6b",
    public_key: KEY,
    name: "Studio",
    platform: "windows",
    fingerprint: "AB:CD",
    trusted_at: 1_758_700_000,
    last_seen: 1_758_700_600,
  },
  connection: { state: "online" },
} as never;

async function phase(backend: MockBackend, next: DictationPhase) {
  await act(async () => {
    backend.publish({ type: "dictation", ...idleDictation(), phase: next });
    await Promise.resolve();
  });
}

async function take(backend: MockBackend, state: PhoneTakeState, opus?: boolean) {
  await act(async () => {
    backend.publish({
      type: "phone_take",
      take: {
        device: KEY,
        take: 1,
        started_at: Date.now() - 3_000,
        state,
        ...(opus === undefined ? {} : { opus }),
      },
    });
    await Promise.resolve();
  });
}

const stateLine = () => screen.getByTestId("phone-mic-state");

/** The core takes `dictation_*` without running a take of its own: the test reports the phases. */
function quiet(backend: MockBackend) {
  const real = backend.invoke.bind(backend);
  return jest
    .spyOn(backend, "invoke")
    .mockImplementation((name: string, ...rest: unknown[]) =>
      name.startsWith("dictation_")
        ? Promise.resolve()
        : (real as (n: string, ...r: unknown[]) => Promise<void>)(name, ...rest),
    );
}

describe("the talk card's lines", () => {
  const t = zhT.t;

  it("words every state of a take sent to a computer", () => {
    expect(phoneTakeLine({ state: "starting" }, 0, t)).toBe("正在打开麦克风…");
    expect(phoneTakeLine({ state: "listening" }, 65_000, t)).toBe("正在录音 · 01:05");
    expect(phoneTakeLine({ state: "processing" }, 0, t)).toBe("电脑正在识别…");
    expect(phoneTakeLine({ state: "done", text: "好的", pasted: true }, 0, t)).toBe(
      "已插入电脑：好的",
    );
    expect(phoneTakeLine({ state: "done", text: "好的", pasted: false }, 0, t)).toBe(
      "已放到电脑剪贴板：好的",
    );
    expect(phoneTakeLine({ state: "failed", code: "busy", message: "", text: "  " }, 0, t)).toBe(
      "电脑正在听写，稍后再试",
    );
    expect(phoneTakeLine({ state: "cancelled" }, 0, t)).toBe("已取消");
  });

  it("words every phase of a take the phone recognises", () => {
    expect(localTakeLine({ phase: "idle" }, 0, t)).toBe("");
    expect(
      localTakeLine({ phase: "listening", started_at: 0, ready: true, locked: false }, 3_000, t),
    ).toBe("正在录音 · 00:03");
    const processing = (stage: "transcribing" | "refining" | "inserting") =>
      localTakeLine({ phase: "processing", stage, started_at: 0 }, 0, t);
    expect(processing("transcribing")).toBe("正在识别…");
    expect(processing("refining")).toBe("正在润色…");
    expect(processing("inserting")).toBe("正在处理…");
    expect(
      localTakeLine(
        {
          phase: "done",
          text: "好的",
          raw_text: "好的",
          chars: 2,
          via: "clipboard",
          refined: false,
          duration_ms: 1_000,
          asr_ms: 100,
          mode: "whole_take",
        },
        0,
        t,
      ),
    ).toBe("已复制到剪贴板：好的");
    expect(localTakeLine({ phase: "failed", message: "", code: "no_speech" }, 0, t)).toBe(
      "没有听到声音",
    );
    expect(localTakeLine({ phase: "failed", message: "asr: 401", code: "asr" }, 0, t)).toMatch(
      /^未完成：/,
    );
    expect(localTakeLine({ phase: "cancelled", injected_chars: 0 }, 0, t)).toBe("已取消");
  });

  it("maps a level in dBFS onto the wave and clamps it", () => {
    expect(levelFraction(-90)).toBe(0);
    expect(levelFraction(-30)).toBe(0.5);
    expect(levelFraction(6)).toBe(1);
  });
});

describe("the talk card on the page", () => {
  it("follows a take the phone recognises: its time and wave while it records, then each outcome", async () => {
    const { backend } = await renderApp();
    const invoke = quiet(backend);
    const button = await screen.findByTestId("phone-mic-hold");
    await fireEvent(button, "responderGrant", touch({ x: 200, y: 500 }));
    await phase(backend, {
      phase: "listening",
      started_at: Date.now(),
      ready: true,
      locked: false,
    });
    expect(await screen.findByTestId("phone-mic-wave")).toBeOnTheScreen();
    expect(screen.getByTestId("phone-mic-eyebrow")).toHaveTextContent(/^正在录音 · /);
    // The take's capture feeds the wave.
    await waitFor(() => {
      expect(screen.getByTestId("phone-mic-wave").props.accessibilityValue.now).toBeGreaterThan(0);
    });
    await fireEvent(button, "responderRelease", touch({ x: 200, y: 500 }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictation_stop");
    });
    await phase(backend, { phase: "processing", stage: "refining", started_at: Date.now() });
    expect(await screen.findByText("正在润色…")).toBeOnTheScreen();
    expect(stateLine()).toHaveAccessibleName("processing");
    await phase(backend, { phase: "failed", message: "", code: "no_speech" });
    expect(await screen.findByText("没有听到声音")).toBeOnTheScreen();
    await phase(backend, { phase: "cancelled", injected_chars: 0 });
    expect(await screen.findByText("已取消")).toBeOnTheScreen();
    expect(screen.getByTestId("phone-mic-eyebrow")).toHaveTextContent("用手机说话");
  });

  it("records without a wave when the meter cannot start", async () => {
    const { backend } = await renderApp();
    quiet(backend);
    jest.spyOn(backend, "meter").mockRejectedValueOnce(new Error("no meter"));
    const button = await screen.findByTestId("phone-mic-hold");
    await fireEvent(button, "responderGrant", touch({ x: 200, y: 500 }));
    await phase(backend, {
      phase: "listening",
      started_at: Date.now(),
      ready: true,
      locked: false,
    });
    expect(await screen.findByTestId("phone-mic-wave")).toBeOnTheScreen();
    expect(screen.getByTestId("phone-mic-wave").props.accessibilityValue.now).toBe(0);
    await fireEvent(button, "responderTerminate");
  });

  it("lets go of a tapped take that ends on its own", async () => {
    const { backend } = await renderApp();
    await fireEvent.press(await screen.findByTestId("phone-talk-mode-tap"));
    await waitFor(() => {
      expect(backend.peek().settings.phone_talk).toBe("tap");
    });
    const button = screen.getByTestId("phone-mic-hold");
    await fireEvent(button, "responderGrant", touch({ x: 200, y: 500 }));
    await phase(backend, {
      phase: "listening",
      started_at: Date.now(),
      ready: true,
      locked: false,
    });
    expect(screen.getByTestId("phone-mic-label")).toHaveTextContent("再次点按，开始识别");
    // The longest recording was reached: the core stopped the take by itself.
    await phase(backend, { phase: "idle" });
    await waitFor(() => {
      expect(screen.getByTestId("phone-mic-label")).toHaveTextContent("点按开始说话");
    });
  });

  it("says why the talk mode or the scene did not change, and clears the scene", async () => {
    const { backend } = await renderApp();
    await screen.findByTestId("phone-mic");
    const invoke = jest.spyOn(backend, "invoke");
    invoke.mockRejectedValueOnce(new Error("设置写不进去"));
    await fireEvent.press(screen.getByTestId("phone-talk-mode-tap"));
    expect(await screen.findByText(/设置写不进去/)).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("phone-scene"));
    const sheet = within(await screen.findByTestId("phone-scene-sheet"));
    await fireEvent.press(sheet.getByRole("radio", { name: "编程开发" }));
    await waitFor(() => {
      expect(backend.peek().settings.pinned_scene).toBeDefined();
    });
    await fireEvent.press(screen.getByTestId("phone-scene"));
    await fireEvent.press(
      within(await screen.findByTestId("phone-scene-sheet")).getByRole("radio", {
        name: "不使用场景",
      }),
    );
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("settings_set_pinned_scene", { id: null });
    });
    invoke.mockRejectedValueOnce(new Error("场景写不进去"));
    await fireEvent.press(screen.getByTestId("phone-scene"));
    await fireEvent.press(
      within(await screen.findByTestId("phone-scene-sheet")).getByRole("radio", { name: "金融" }),
    );
    expect(await screen.findByText(/场景写不进去/)).toBeOnTheScreen();
  });

  it("follows a take sent to a computer, its Opus stream and each outcome", async () => {
    const { backend } = await renderApp({ mock: { devices: [STUDIO] } });
    await screen.findByTestId("phone-route");
    await take(backend, { state: "starting" });
    expect(await screen.findByText("正在打开麦克风…")).toBeOnTheScreen();
    await take(backend, { state: "listening" }, true);
    expect(await screen.findByText("Opus 压缩传输 · 约 24 kbit/s")).toBeOnTheScreen();
    expect(screen.getByTestId("phone-mic-eyebrow")).toHaveTextContent(/^正在录音 · 00:0\d$/);
    await take(backend, { state: "processing" }, true);
    expect(await screen.findByText("电脑正在识别…")).toBeOnTheScreen();
    await take(backend, { state: "done", text: "会议改到三点", pasted: false });
    expect(await screen.findByText("已放到电脑剪贴板：会议改到三点")).toBeOnTheScreen();
    expect(screen.queryByText("Opus 压缩传输 · 约 24 kbit/s")).toBeNull();
    await take(backend, { state: "failed", code: "failed", message: "x", text: "会议改到三点" });
    expect(await screen.findByText(/^未能插入电脑当前的窗口/)).toBeOnTheScreen();
    await take(backend, { state: "failed", code: "offline", message: "" });
    expect(await screen.findByText("电脑已离线")).toBeOnTheScreen();
    await take(backend, { state: "cancelled" });
    expect(await screen.findByText("已取消")).toBeOnTheScreen();
  });

  it("picks the computer among several online, and says when the start was refused", async () => {
    const LAPTOP = {
      ...(STUDIO as { device: object; connection: object }),
      device: {
        ...(STUDIO as { device: object }).device,
        device_id: "0f3f1a1e-8d4b-4c8e-9f7a-1c2d3e4f5a6c",
        public_key: "2".repeat(64),
        name: "笔记本",
      },
    } as never;
    const { backend } = await renderApp({ mock: { devices: [STUDIO, LAPTOP] } });
    await fireEvent.press(await screen.findByTestId("phone-mic-target"));
    await fireEvent.press(
      within(await screen.findByTestId("phone-mic-target-sheet")).getByRole("radio", {
        name: "笔记本",
      }),
    );
    expect(await screen.findByText("发送到 笔记本")).toBeOnTheScreen();
    const invoke = jest.spyOn(backend, "invoke").mockRejectedValueOnce("电脑正忙");
    const button = screen.getByTestId("phone-mic-hold");
    await fireEvent(button, "responderGrant", touch({ x: 200, y: 500 }));
    expect(invoke).toHaveBeenCalledWith("phone_take_start", { publicKey: "2".repeat(64) });
    expect(await screen.findByText(/电脑正忙/)).toBeOnTheScreen();
    await fireEvent(button, "responderRelease", touch({ x: 200, y: 500 }));
    expect(invoke).not.toHaveBeenCalledWith("phone_take_stop");
  });
});
