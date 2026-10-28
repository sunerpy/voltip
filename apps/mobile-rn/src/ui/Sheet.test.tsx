// The bottom sheet on its own (docs/mobile-rn.md §5): a swipe down on its handle takes it away when
// it goes far or fast enough, and springs it back otherwise. The handle only claims a gesture that
// moves (no `onStartShouldSetResponder`), which `fireEvent` takes for a view that cannot be
// touched, so the tests call the responder handlers the way the responder system does.
import { act, render, screen } from "@testing-library/react-native";
import { Text } from "react-native";
import { Provider as PaperProvider } from "react-native-paper";
import { SafeAreaProvider } from "react-native-safe-area-context";

import { METRICS } from "../test/render";
import { appTheme } from "../theme/themes";
import { BottomSheet } from "./Sheet";

/** A one-finger touch that moved from (0, 0) at time 0 to `(dx, dy)` at `at` ms. */
function moved(dx: number, dy: number, at: number) {
  const touch = {
    touchActive: true,
    startPageX: 0,
    startPageY: 0,
    startTimeStamp: 0,
    previousPageX: 0,
    previousPageY: 0,
    previousTimeStamp: 0,
    currentPageX: dx,
    currentPageY: dy,
    currentTimeStamp: at,
  };
  return {
    nativeEvent: { touches: [touch] },
    touchHistory: {
      numberActiveTouches: 1,
      indexOfSingleActiveTouch: 0,
      mostRecentTimeStamp: at,
      touchBank: [touch],
    },
  };
}

type Handlers = Record<
  | "onMoveShouldSetResponderCapture"
  | "onMoveShouldSetResponder"
  | "onResponderGrant"
  | "onResponderMove"
  | "onResponderRelease"
  | "onResponderTerminate",
  (event: ReturnType<typeof moved>) => unknown
>;

async function sheet(close: () => void): Promise<Handlers> {
  await render(
    <SafeAreaProvider initialMetrics={METRICS}>
      <PaperProvider theme={appTheme("light")}>
        <BottomSheet open close={close} title="面板" testID="panel">
          <Text>内容</Text>
        </BottomSheet>
      </PaperProvider>
    </SafeAreaProvider>,
  );
  return screen.getByTestId("panel-handle").props as Handlers;
}

/** A finger down on the handle, moved to `to`, then let go (or taken by another view). */
async function swipe(handle: Handlers, to: ReturnType<typeof moved>, end: "release" | "terminate") {
  await act(async () => {
    handle.onResponderGrant(moved(0, 0, 0));
    handle.onResponderMove(to);
    if (end === "release") handle.onResponderRelease(to);
    else handle.onResponderTerminate(to);
    await Promise.resolve();
  });
}

describe("BottomSheet", () => {
  it("claims a downward move on its handle, not a sideways or tiny one", async () => {
    const handle = await sheet(jest.fn());
    const ask = (event: ReturnType<typeof moved>) => {
      // The capture phase measures the move; the bubble phase decides.
      handle.onMoveShouldSetResponderCapture(event);
      return handle.onMoveShouldSetResponder(event);
    };
    expect(ask(moved(2, 3, 10))).toBe(false);
    expect(ask(moved(60, 30, 20))).toBe(false);
    expect(ask(moved(0, 90, 30))).toBe(true);
  });

  it("closes after a long swipe or a fast flick, and springs back from a slow short drag or a lost gesture", async () => {
    const close = jest.fn();
    const handle = await sheet(close);
    await swipe(handle, moved(0, 120, 1_000), "release");
    expect(close).toHaveBeenCalledTimes(1);
    await swipe(handle, moved(0, 40, 20), "release");
    expect(close).toHaveBeenCalledTimes(2);
    await swipe(handle, moved(0, 30, 2_000), "release");
    await swipe(handle, moved(0, 120, 100), "terminate");
    expect(close).toHaveBeenCalledTimes(2);
  });
});
