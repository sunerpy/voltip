// The app on a wide window (docs/mobile-rn.md §5): a tablet, an unfolded phone, a phone on its side.
// A page keeps a phone's reading width, centred, clear of the system's side insets (a phone on its
// side has its navigation bar and camera cutout there); the tab dock keeps a phone's width; a sheet
// keeps 640 dp, stays clear of the side insets and leaves the status bar free above it.
import { act, render, screen } from "@testing-library/react-native";
import { Dimensions, StyleSheet, Text, type ViewStyle } from "react-native";
import { Provider as PaperProvider } from "react-native-paper";
import { type Metrics, SafeAreaProvider } from "react-native-safe-area-context";

import { METRICS, openTab, renderApp } from "../test/render";
import { appTheme } from "../theme/themes";
import { PAGE_MAX_WIDTH, Page } from "./kit";
import { BottomSheet } from "./Sheet";

/** A phone on its side: the navigation bar on the right, the camera cutout on the left. */
const LANDSCAPE: Metrics = {
  frame: { x: 0, y: 0, width: 915, height: 412 },
  insets: { top: 24, right: 48, bottom: 0, left: 32 },
};

const before = Dimensions.get("window");
/** The window (and screen) `metrics` names, as Android reports it after a rotation. */
async function windowOf(metrics: Metrics) {
  const size = { ...before, width: metrics.frame.width, height: metrics.frame.height };
  await act(async () => {
    Dimensions.set({ window: size, screen: size });
    await Promise.resolve();
  });
}
afterEach(async () => {
  await act(async () => {
    Dimensions.set({ window: before, screen: before });
    await Promise.resolve();
  });
});

function style(element: { props: { style?: unknown } } | null | undefined): ViewStyle {
  return StyleSheet.flatten(element?.props.style as ViewStyle) ?? {};
}

describe("a wide window", () => {
  it("keeps a page's column at a phone's reading width, centred and clear of the side insets", async () => {
    await windowOf(LANDSCAPE);
    await render(
      <SafeAreaProvider initialMetrics={LANDSCAPE}>
        <PaperProvider theme={appTheme("light")}>
          <Page testID="page">
            <Text>内容</Text>
          </Page>
        </PaperProvider>
      </SafeAreaProvider>,
    );
    const column = screen.getByTestId("page-column");
    expect(column).toHaveStyle({ width: "100%", maxWidth: PAGE_MAX_WIDTH, alignSelf: "center" });
    // The scroll view's content around it: 16 dp from each side's inset.
    const content = StyleSheet.flatten(
      screen.getByTestId("page").props.contentContainerStyle as ViewStyle,
    );
    expect(content).toMatchObject({ paddingLeft: 16 + 32, paddingRight: 16 + 48 });
  });

  it("keeps the tab dock at a phone's width, clear of the side insets", async () => {
    await windowOf(LANDSCAPE);
    await renderApp({ metrics: LANDSCAPE });
    await openTab("settings");
    const dock = await screen.findByTestId("tab-dock");
    expect(dock).toHaveStyle({ width: "100%", maxWidth: 480, alignSelf: "center" });
    // 24 dp from each side's inset.
    expect(style(dock.parent)).toMatchObject({ paddingLeft: 24 + 32, paddingRight: 24 + 48 });
    // The settings page's column under it is a phone's reading width too.
    expect(await screen.findByTestId("phone-settings-column")).toHaveStyle({ maxWidth: 720 });
  });

  it("opens a sheet clear of the side insets, under the status bar, at most 640 dp wide", async () => {
    await windowOf(LANDSCAPE);
    await render(
      <SafeAreaProvider initialMetrics={LANDSCAPE}>
        <PaperProvider theme={appTheme("light")}>
          <BottomSheet open close={() => undefined} title="面板" maxHeight={0.95} testID="panel">
            <Text>内容</Text>
          </BottomSheet>
        </PaperProvider>
      </SafeAreaProvider>,
    );
    const sheet = screen.getByTestId("panel-sheet");
    // 95 % of 412 dp would run under the status bar: the sheet stops 8 dp below it.
    expect(sheet).toHaveStyle({ maxWidth: 640, maxHeight: 412 - 24 - 8 });
    expect(style(sheet.parent)).toMatchObject({ paddingLeft: 32, paddingRight: 48 });
  });

  it("leaves a phone's portrait sheet as tall as it asks", async () => {
    await windowOf(METRICS);
    await render(
      <SafeAreaProvider initialMetrics={METRICS}>
        <PaperProvider theme={appTheme("light")}>
          <BottomSheet open close={() => undefined} title="面板" testID="panel">
            <Text>内容</Text>
          </BottomSheet>
        </PaperProvider>
      </SafeAreaProvider>,
    );
    expect(screen.getByTestId("panel-sheet")).toHaveStyle({ maxHeight: 915 * 0.8 });
  });
});
