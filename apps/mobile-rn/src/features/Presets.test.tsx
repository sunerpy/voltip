// 预设 on the phone's AI 模型 page (docs/dictation.md §21): the built-in presets to use or copy, the
// custom ones to use, edit or delete after asking, and the editor's checks and 试运行.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { MockBackend } from "@voltip/shared/mock";

import { hold, openTab, renderApp } from "../test/render";

async function openPresets() {
  await openTab("settings");
  await fireEvent.press(await screen.findByTestId("settings-ai"));
  return screen.findByTestId("presets-section");
}

async function seed(backend: MockBackend, name: string, prompt: string) {
  await act(async () => {
    await backend.invoke("presets_add", { preset: { name, prompt } });
  });
}

describe("预设 on the AI 模型 page", () => {
  it("uses a built-in preset, and copies one into the editor to save as a custom preset", async () => {
    const { backend } = await renderApp();
    const section = await openPresets();
    expect(within(section).getByTestId("presets-current")).toHaveTextContent("当前：校对");
    expect(within(section).getByTestId("presets-empty")).toBeOnTheScreen();
    await fireEvent.press(within(section).getByTestId("preset-chat"));
    await waitFor(() => {
      expect(backend.peek().settings.engines.refine_preset).toBe("chat");
    });
    expect(within(section).getByTestId("presets-current")).toHaveTextContent("当前：口语聊天");
    expect(within(section).getByTestId("preset-chat")).toHaveTextContent(/口语聊天\s+使用中/);
    // Choosing the preset in use again asks nothing of the core.
    const invoke = jest.spyOn(backend, "invoke");
    await fireEvent.press(within(section).getByTestId("preset-chat"));
    expect(invoke).not.toHaveBeenCalled();
    invoke.mockRestore();
    await fireEvent.press(within(section).getByLabelText("复制为自定义 · 书面语"));
    const editor = await screen.findByTestId("preset-editor");
    expect(within(editor).getByTestId("preset-name")).toHaveProp("value", "书面语（副本）");
    expect(within(editor).getByTestId("preset-prompt").props.value).not.toBe("");
    await fireEvent.press(within(editor).getByTestId("dialog-save"));
    expect(await screen.findByText("已保存预设 · 书面语（副本）")).toBeOnTheScreen();
    expect(backend.peek().presets.map((p) => p.name)).toEqual(["书面语（副本）"]);
    expect(within(section).queryByTestId("presets-empty")).toBeNull();
  });

  it("creates, edits and deletes a custom preset, with the editor's checks", async () => {
    const { backend } = await renderApp();
    await seed(backend, "周报", "整理成周报。");
    const section = await openPresets();
    await fireEvent.press(within(section).getByTestId("presets-add"));
    let editor = await screen.findByTestId("preset-editor");
    expect(within(editor).getByText("新建预设")).toBeOnTheScreen();
    // Nothing is said about a field left empty until the first save.
    expect(within(editor).queryByText("请填写名称")).toBeNull();
    await fireEvent.press(within(editor).getByTestId("dialog-save"));
    expect(within(editor).getByText("请填写名称")).toBeOnTheScreen();
    await fireEvent.changeText(within(editor).getByTestId("preset-name"), "周报");
    expect(within(editor).getByText("已有名为「周报」的预设")).toBeOnTheScreen();
    await fireEvent.changeText(within(editor).getByTestId("preset-name"), "  日报 ");
    await fireEvent.changeText(within(editor).getByTestId("preset-prompt"), "整理成日报。");
    expect(within(editor).getByText(/^6 \/ \d+$/)).toBeOnTheScreen();
    await fireEvent.press(within(editor).getByTestId("dialog-save"));
    expect(await screen.findByText("已保存预设 · 日报")).toBeOnTheScreen();
    expect(backend.peek().presets.map((p) => p.name)).toEqual(["周报", "日报"]);

    await fireEvent.press(within(section).getByLabelText("编辑 周报"));
    editor = await screen.findByTestId("preset-editor");
    expect(within(editor).getByText("编辑预设")).toBeOnTheScreen();
    await fireEvent.changeText(within(editor).getByTestId("preset-prompt"), "整理成周报，分三段。");
    await fireEvent.press(within(editor).getByTestId("dialog-save"));
    await waitFor(() => {
      expect(backend.peek().presets[0]?.prompt).toBe("整理成周报，分三段。");
    });
    // A custom preset in use names itself; deleting one asks first.
    await fireEvent.press(within(section).getByLabelText("日报"));
    await waitFor(() => {
      expect(within(section).getByTestId("presets-current")).toHaveTextContent("当前：日报");
    });
    await fireEvent.press(within(section).getByLabelText("删除 周报"));
    expect(await screen.findByText("删除预设「周报」？")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("shell-confirm"));
    expect(await screen.findByText("已删除预设 · 周报")).toBeOnTheScreen();
    expect(backend.peek().presets.map((p) => p.name)).toEqual(["日报"]);
  });

  it("tries the prompt on a sample text and shows the answer, the wait or why it failed", async () => {
    const { backend } = await renderApp();
    const section = await openPresets();
    await fireEvent.press(within(section).getByTestId("presets-add"));
    const editor = await screen.findByTestId("preset-editor");
    const trial = within(editor).getByTestId("preset-trial");
    expect(
      within(trial).getByText("用当前的 AI 服务处理一段示例文字，结果不会保存。"),
    ).toBeOnTheScreen();
    await fireEvent.changeText(within(editor).getByTestId("preset-prompt"), "只加标点。");
    // The core's answer is held, so the wait is on the screen however slow the machine is.
    const invoke = hold(backend, "presets_try");
    await fireEvent.press(within(editor).getByTestId("preset-try"));
    expect(within(editor).getByText("正在处理…")).toBeOnTheScreen();
    const [, asked] = invoke.mock.calls.find(([name]) => name === "presets_try") ?? [];
    invoke.mockRestore();
    await act(async () => {
      backend.publish({
        type: "preset_try",
        id: (asked as { id: number }).id,
        outcome: {
          status: "ok",
          text: "今天天气不错。",
          latency_ms: 300,
          model: "qwen/qwen3.8-27b",
        },
      });
      await Promise.resolve();
    });
    expect(await within(editor).findByTestId("preset-trial-result")).toHaveTextContent(/结果 · /);
    // A sample cleared away cannot be tried; a new one clears the old answer.
    await fireEvent.changeText(within(editor).getByTestId("preset-sample"), "   ");
    expect(within(editor).queryByTestId("preset-trial-result")).toBeNull();
    expect(within(editor).getByTestId("preset-try")).toBeDisabled();
    await fireEvent.changeText(within(editor).getByTestId("preset-sample"), "今天天气不错");
    jest.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("服务不可用"));
    await fireEvent.press(within(editor).getByTestId("preset-try"));
    expect(await within(editor).findByTestId("preset-trial-failed")).toHaveTextContent(
      "试运行失败：服务不可用",
    );
    // The core refuses the save: the reason stays in the editor.
    await fireEvent.changeText(within(editor).getByTestId("preset-name"), "标点");
    jest.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("预设已满"));
    await fireEvent.press(within(editor).getByTestId("dialog-save"));
    expect(await within(editor).findByTestId("preset-save-error")).toHaveTextContent("预设已满");
    await fireEvent.changeText(within(editor).getByTestId("preset-name"), "标点符号");
    expect(within(editor).queryByTestId("preset-save-error")).toBeNull();
    await fireEvent.press(within(editor).getByLabelText("关闭"));
    await waitFor(() => {
      expect(screen.queryByTestId("preset-editor")).toBeNull();
    });
    expect(backend.peek().presets).toEqual([]);
  });

  it("says a try is not possible without a clean-up service, and names a deleted preset in use", async () => {
    const backend = new MockBackend({ role: "phone", builtIn: {} });
    const state = backend.peek();
    await backend.invoke("settings_set_engines", {
      engines: { ...state.settings.engines, refine_preset: "11111111-1111-4111-8111-111111111111" },
    });
    await renderApp({ backend });
    const section = await openPresets();
    expect(within(section).getByTestId("presets-missing")).toHaveTextContent(
      "已删除的预设（按校对处理）",
    );
    await fireEvent.press(within(section).getByTestId("presets-add"));
    const editor = await screen.findByTestId("preset-editor");
    expect(within(editor).getByText("尚未配置 AI 润色服务，无法试运行。")).toBeOnTheScreen();
    expect(within(editor).getByTestId("preset-try")).toBeDisabled();
  });

  it("says why a copy or a change of preset failed", async () => {
    const { backend } = await renderApp();
    const section = await openPresets();
    jest.spyOn(backend, "presetsBuiltin").mockRejectedValueOnce(new Error("读不出内置预设"));
    await fireEvent.press(within(section).getByLabelText("复制为自定义 · 校对"));
    expect(await screen.findByText("读不出内置预设")).toBeOnTheScreen();
    expect(screen.queryByTestId("preset-editor")).toBeNull();
    jest.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("设置写不进去"));
    await fireEvent.press(within(section).getByTestId("preset-notes"));
    expect(await screen.findByText("设置写不进去")).toBeOnTheScreen();
    expect(backend.peek().settings.engines.refine_preset).toBe("proofread");
  });
});
