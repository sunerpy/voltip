// 设置 › 场景 on the phone (docs/dictation.md §18): the built-in scenes with their term packs and the
// user's own, picked by hand on the talk card, so a scene lists no applications here.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import type { MockBackend } from "@voltip/shared/mock";

import { openTab, renderApp } from "../test/render";

async function openScenes() {
  await openTab("settings");
  await fireEvent.press(await screen.findByTestId("settings-scenes"));
  return screen.findByTestId("phone-scenes");
}

async function seed(backend: MockBackend, name: string) {
  await act(async () => {
    await backend.invoke("scenes_add", {
      scene: { name, enabled: true, match: { apps: [], title_contains: [] }, overrides: {} },
    });
  });
}

async function choose(editor: ReturnType<typeof within>, field: string, value: string) {
  await fireEvent.press(editor.getByTestId(field));
  await fireEvent.press(await screen.findByTestId(`${field}-option-${value}`));
}

/** The card of the scene `name`. */
function card(name: string) {
  const found = screen
    .getAllByTestId("scene-card")
    .find((c) => within(c).queryByLabelText(`编辑 ${name}`) !== null);
  if (found === undefined) throw new Error(`no card for ${name}`);
  return within(found);
}

describe("设置 › 场景", () => {
  it("lists the built-in scenes with their terms, and saves a new scene with its overrides", async () => {
    const { backend } = await renderApp();
    await seed(backend, "客户邮件");
    await openScenes();
    const chat = card("即时聊天");
    expect(chat.getByText("内置")).toBeOnTheScreen();
    expect(chat.getByTestId("scene-builtin-description")).toHaveTextContent(
      "聊天软件：口语化的短句。",
    );
    expect(chat.queryByLabelText("删除 即时聊天")).toBeNull();
    // 即时聊天 has no term pack; 编程开发 has one.
    expect(chat.queryByTestId("scene-terms")).toBeNull();
    const coding = card("编程开发");
    expect(await coding.findByTestId("scene-terms")).toHaveTextContent(/术语 \d+ 个/);
    await fireEvent.press(coding.getByLabelText("查看 编程开发 的术语"));
    expect(await screen.findByText("编程开发 · 术语")).toBeOnTheScreen();
    expect(screen.getByTestId("scene-terms-list").children.length).toBeGreaterThan(0);
    await fireEvent.press(screen.getByTestId("scene-terms-close"));
    await waitFor(() => {
      expect(screen.queryByText("编程开发 · 术语")).toBeNull();
    });

    await fireEvent.press(screen.getByTestId("scenes-add"));
    const editor = within(await screen.findByTestId("scene-editor"));
    expect(editor.getByText("新建场景")).toBeOnTheScreen();
    expect(editor.queryByText("请填写名称")).toBeNull();
    await fireEvent.press(editor.getByTestId("dialog-save"));
    expect(editor.getByText("请填写名称")).toBeOnTheScreen();
    await fireEvent.changeText(editor.getByTestId("scene-name"), "客户邮件");
    expect(editor.getByText("已有名为「客户邮件」的场景")).toBeOnTheScreen();
    await fireEvent.changeText(editor.getByTestId("scene-name"), " 会议纪要 ");
    await choose(editor, "scene-refine", "on");
    await choose(editor, "scene-preset", "notes");
    await choose(editor, "scene-language", "en");
    await choose(editor, "scene-script", "traditional");
    await fireEvent.changeText(editor.getByTestId("scene-prompt"), "整理成会议纪要");
    expect(editor.getByTestId("scene-prompt-count")).toHaveTextContent(/^7 \//);
    await fireEvent.press(editor.getByTestId("dialog-save"));
    expect(await screen.findByText("已保存场景 · 会议纪要")).toBeOnTheScreen();
    const saved = backend.peek().scenes.find((s) => s.name === "会议纪要");
    expect(saved?.builtin).toBeUndefined();
    expect(saved).toMatchObject({
      match: { apps: [] },
      overrides: {
        refine_enabled: true,
        refine_preset: "notes",
        language: "en",
        chinese_script: "traditional",
        prompt: "整理成会议纪要",
      },
    });
    expect(card("会议纪要").getByTestId("scene-summary")).toHaveTextContent(
      /AI 润色 开 · AI 预设：要点纪要 · .* · 有补充要求/,
    );
  });

  // Regression (2026-10-28): the notice named a built-in scene by its stored category (`chat`).
  it("edits a built-in scene, restores its defaults after asking, and deletes one of the user's", async () => {
    const { backend } = await renderApp();
    await seed(backend, "客户邮件");
    await openScenes();
    expect(card("客户邮件").getByTestId("scene-summary")).toHaveTextContent("全部跟随全局设置");
    await fireEvent.press(screen.getByLabelText("编辑 即时聊天"));
    let editor = within(await screen.findByTestId("scene-editor"));
    expect(editor.getByText("编辑场景")).toBeOnTheScreen();
    expect(editor.getByTestId("scene-builtin-name")).toHaveProp("value", "即时聊天");
    expect(editor.getByText("内置场景的名称不能修改。")).toBeOnTheScreen();
    await fireEvent.changeText(editor.getByTestId("scene-prompt"), "句末不加句号");
    await fireEvent.press(editor.getByTestId("dialog-save"));
    expect(await screen.findByText("已保存场景 · 即时聊天")).toBeOnTheScreen();
    const chat = () => backend.peek().scenes.find((s) => s.builtin === "chat");
    expect(chat()?.overrides.prompt).toBe("句末不加句号");

    await fireEvent.press(screen.getByLabelText("编辑 即时聊天"));
    editor = within(await screen.findByTestId("scene-editor"));
    await fireEvent.press(editor.getByTestId("scene-restore"));
    expect(await screen.findByText("恢复「即时聊天」的默认设置？")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("shell-confirm"));
    expect(await screen.findByText("已恢复默认 · 即时聊天")).toBeOnTheScreen();
    expect(chat()?.overrides.prompt).not.toBe("句末不加句号");
    await waitFor(() => {
      expect(screen.queryByTestId("scene-editor")).toBeNull();
    });

    await fireEvent.press(card("客户邮件").getByLabelText("删除 客户邮件"));
    expect(await screen.findByText("删除场景「客户邮件」？")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("shell-confirm"));
    await waitFor(() => {
      expect(backend.peek().scenes.some((s) => s.name === "客户邮件")).toBe(false);
    });
  });

  it("says why a save, a restore or a deletion did not go through", async () => {
    const { backend } = await renderApp();
    await seed(backend, "客户邮件");
    await openScenes();
    await fireEvent.press(screen.getByLabelText("编辑 客户邮件"));
    let editor = within(await screen.findByTestId("scene-editor"));
    const invoke = jest.spyOn(backend, "invoke");
    invoke.mockRejectedValueOnce(new Error("场景写不进去"));
    await fireEvent.press(editor.getByTestId("dialog-save"));
    expect(await editor.findByTestId("scene-save-error")).toHaveTextContent("场景写不进去");
    await fireEvent.changeText(editor.getByTestId("scene-prompt"), "正式一些");
    expect(editor.queryByTestId("scene-save-error")).toBeNull();
    await fireEvent.press(editor.getByLabelText("关闭"));
    await waitFor(() => {
      expect(screen.queryByTestId("scene-editor")).toBeNull();
    });

    await fireEvent.press(screen.getByLabelText("编辑 金融"));
    editor = within(await screen.findByTestId("scene-editor"));
    invoke.mockRejectedValueOnce(new Error("恢复失败"));
    await fireEvent.press(editor.getByTestId("scene-restore"));
    await fireEvent.press(await screen.findByTestId("shell-confirm"));
    expect(await editor.findByTestId("scene-save-error")).toHaveTextContent("恢复失败");
    await fireEvent.press(editor.getByLabelText("关闭"));
    await waitFor(() => {
      expect(screen.queryByTestId("scene-editor")).toBeNull();
    });

    invoke.mockRejectedValueOnce(new Error("删除失败"));
    await fireEvent.press(screen.getByLabelText("删除 客户邮件"));
    await fireEvent.press(await screen.findByTestId("shell-confirm"));
    expect(await screen.findByText(/删除失败/)).toBeOnTheScreen();
    expect(backend.peek().scenes.some((s) => s.name === "客户邮件")).toBe(true);
    invoke.mockRestore();
  });
});
