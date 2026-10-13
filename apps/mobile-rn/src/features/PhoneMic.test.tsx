// The phone's talk card (docs/dictation.md §20.11): holding or tapping (user request 2026-10-13),
// how far a held finger may slip before letting go cancels, and how a take the computer recognised
// but could not insert reads.
import { act, fireEvent, screen, waitFor } from "@testing-library/react-native";
import { zhT } from "@voltip/shared";

import { openTab, renderApp } from "../test/render";
import { CANCEL_DISTANCE, phoneTakeLine } from "./PhoneMic";

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
