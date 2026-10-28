// 设置 › 历史记录 on the phone (docs/dictation.md §4, §20.7): whether takes are recorded, how many are
// kept, and 清空历史 after asking.
import { fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { type HistoryEntry, defaultSettings } from "@voltip/shared";

import { openTab, renderApp } from "../test/render";

function entry(n: number): HistoryEntry {
  return {
    id: `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`,
    at_ms: Date.now() - n * 60_000,
    raw_text: `第 ${n} 条`,
    text: `第 ${n} 条。`,
    refined: false,
    asr_model: "whisper-large-v3",
    duration_ms: 3_000,
    asr_ms: 200,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
  };
}

async function openFromHistory() {
  await openTab("history");
  await fireEvent.press(await screen.findByTestId("phone-history-retention"));
  return within(await screen.findByTestId("phone-history-settings"));
}

describe("设置 › 历史记录", () => {
  it("stops recording, keeps fewer and clears after asking; 记录 says what is kept", async () => {
    const { backend } = await renderApp({ mock: { history: [entry(1), entry(2), entry(3)] } });
    await openTab("history");
    expect(
      await screen.findByText("保留最近 20,000 条，超出后自动删除最早的记录。"),
    ).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("phone-history-retention"));
    const page = within(await screen.findByTestId("phone-history-settings"));
    expect(page.getByText("3 / 20,000 条")).toBeOnTheScreen();
    const record = page.getByRole("switch", { name: /保存听写历史/ });
    expect(record).toBeChecked();
    await fireEvent.press(record);
    await waitFor(() => {
      expect(backend.peek().settings.history).toEqual({ enabled: false, keep: 20_000 });
    });
    expect(page.getByRole("switch", { name: /保存听写历史/ })).not.toBeChecked();

    await fireEvent.press(page.getByRole("button", { name: /保留最近/ }));
    await fireEvent.press(await screen.findByRole("radio", { name: "500 条" }));
    await waitFor(() => {
      expect(backend.peek().settings.history).toEqual({ enabled: false, keep: 500 });
    });
    expect(page.getByText("3 / 500 条")).toBeOnTheScreen();

    await fireEvent.press(page.getByText("清空历史"));
    expect(await screen.findByText("清空 3 条历史记录？")).toBeOnTheScreen();
    expect(screen.getByText("这部手机上的 3 条记录将被删除，此操作无法撤销。")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("shell-confirm"));
    await waitFor(() => {
      expect(page.getByText("0 / 500 条")).toBeOnTheScreen();
    });
    expect(page.getByText("清空历史")).toBeDisabled();
  });

  it("offers a kept count set elsewhere among the choices, and says why a change did not go through", async () => {
    const { backend } = await renderApp({
      mock: { settings: { history: { ...defaultSettings().history, keep: 777 } } },
    });
    const page = await openFromHistory();
    expect(page.getByText("0 / 777 条")).toBeOnTheScreen();
    expect(page.getByText("清空历史")).toBeDisabled();
    await fireEvent.press(page.getByRole("button", { name: /保留最近/ }));
    const choices = (await screen.findAllByRole("radio")).map(
      (r) => r.props.accessibilityLabel as string,
    );
    expect(choices).toEqual(["500 条", "777 条", "2,000 条", "5,000 条", "10,000 条", "20,000 条"]);
    jest.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("设置写不进去"));
    await fireEvent.press(screen.getByRole("radio", { name: "2,000 条" }));
    expect(await screen.findByText(/设置写不进去/)).toBeOnTheScreen();
    expect(backend.peek().settings.history.keep).toBe(777);
  });
});
