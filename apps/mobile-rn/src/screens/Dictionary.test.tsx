// 设置 › 个人词典 on the phone (docs/dictation.md §16): the core's entries in matching order,
// searchable, each switched in its row and edited full screen, where it also moves and is deleted
// after asking.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import type { MockBackend } from "@voltip/shared/mock";

import { openTab, renderApp } from "../test/render";

async function openDictionary() {
  await openTab("settings");
  await fireEvent.press(await screen.findByTestId("settings-dictionary"));
  return screen.findByTestId("phone-dictionary");
}

async function seed(backend: MockBackend, term: string, heard_as: string[] = []) {
  await act(async () => {
    await backend.invoke("dictionary_add", { entry: { term, heard_as, enabled: true } });
  });
}

describe("设置 › 个人词典", () => {
  it("starts empty, and adds an entry with what it was heard as", async () => {
    const { backend } = await renderApp();
    const page = await openDictionary();
    expect(within(page).getByText("暂无词条。")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("dictionary-add"));
    const editor = within(await screen.findByTestId("dictionary-editor"));
    expect(editor.getByText("新建词条")).toBeOnTheScreen();
    expect(editor.getByTestId("dialog-save")).toBeDisabled();
    expect(editor.getByText("多个写法用逗号或 · 分隔")).toBeOnTheScreen();
    await fireEvent.changeText(editor.getByTestId("dictionary-term"), " Voltip ");
    await fireEvent.changeText(editor.getByTestId("dictionary-heard"), "沃尔提普，伏特");
    await fireEvent.press(editor.getByTestId("dialog-save"));
    await waitFor(() => {
      expect(screen.queryByTestId("dictionary-editor")).toBeNull();
    });
    expect(backend.peek().dictionary).toMatchObject([
      { term: "Voltip", heard_as: ["沃尔提普", "伏特"], enabled: true },
    ]);
    expect(within(page).getByLabelText("编辑 Voltip")).toHaveTextContent(/沃尔提普 · 伏特$/);
  });

  it("refuses a written form already there, searches, switches, moves and deletes after asking", async () => {
    const { backend } = await renderApp();
    await seed(backend, "Voltip", ["沃尔提普"]);
    await seed(backend, "飞书");
    const page = await openDictionary();
    await fireEvent.press(screen.getByTestId("dictionary-add"));
    let editor = within(await screen.findByTestId("dictionary-editor"));
    await fireEvent.changeText(editor.getByTestId("dictionary-term"), "飞书");
    expect(editor.getByText("词典里已有这个写法")).toBeOnTheScreen();
    expect(editor.getByTestId("dialog-save")).toBeDisabled();
    await fireEvent.press(editor.getByLabelText("关闭"));
    await waitFor(() => {
      expect(screen.queryByTestId("dictionary-editor")).toBeNull();
    });

    // The search matches either the written form or a heard one.
    await fireEvent.changeText(within(page).getByTestId("dictionary-search"), "沃尔");
    expect(within(page).getByLabelText("编辑 Voltip")).toBeOnTheScreen();
    expect(within(page).queryByLabelText("编辑 飞书")).toBeNull();
    await fireEvent.changeText(within(page).getByTestId("dictionary-search"), "钉钉");
    expect(within(page).getByText("没有匹配的词条")).toBeOnTheScreen();
    await fireEvent.changeText(within(page).getByTestId("dictionary-search"), "");

    await fireEvent(within(page).getByLabelText("启用 飞书"), "valueChange", false);
    await waitFor(() => {
      expect(backend.peek().dictionary[1]?.enabled).toBe(false);
    });
    await fireEvent.press(within(page).getByLabelText("编辑 飞书"));
    editor = within(await screen.findByTestId("dictionary-editor"));
    expect(editor.getByText("编辑词条")).toBeOnTheScreen();
    expect(editor.getByText("匹配顺序第 2 条，共 2 条")).toBeOnTheScreen();
    expect(editor.getByTestId("dictionary-down")).toBeDisabled();
    await fireEvent.press(editor.getByTestId("dictionary-up"));
    await waitFor(() => {
      expect(backend.peek().dictionary.map((e) => e.term)).toEqual(["飞书", "Voltip"]);
    });
    // Saving keeps the entry switched off.
    await fireEvent.changeText(editor.getByTestId("dictionary-heard"), "飞输");
    await fireEvent.press(editor.getByTestId("dialog-save"));
    await waitFor(() => {
      expect(backend.peek().dictionary[0]).toMatchObject({ heard_as: ["飞输"], enabled: false });
    });
    await fireEvent.press(within(page).getByLabelText("编辑 飞书"));
    editor = within(await screen.findByTestId("dictionary-editor"));
    await fireEvent.press(editor.getByTestId("dictionary-delete"));
    expect(await screen.findByText("删除“飞书”？")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("shell-confirm"));
    await waitFor(() => {
      expect(backend.peek().dictionary.map((e) => e.term)).toEqual(["Voltip"]);
    });
    await waitFor(() => {
      expect(screen.queryByTestId("dictionary-editor")).toBeNull();
    });
  });

  it("says why a save, a move or a switch did not go through", async () => {
    const { backend } = await renderApp();
    await seed(backend, "Voltip");
    await seed(backend, "飞书");
    const page = await openDictionary();
    const invoke = jest.spyOn(backend, "invoke");
    invoke.mockRejectedValueOnce(new Error("词典写不进去"));
    await fireEvent(within(page).getByLabelText("启用 Voltip"), "valueChange", false);
    expect(await screen.findByText(/词典写不进去/)).toBeOnTheScreen();
    await fireEvent.press(within(page).getByLabelText("编辑 Voltip"));
    const editor = within(await screen.findByTestId("dictionary-editor"));
    invoke.mockRejectedValueOnce(new Error("顺序没变"));
    await fireEvent.press(editor.getByTestId("dictionary-down"));
    expect(await screen.findByText(/顺序没变/)).toBeOnTheScreen();
    invoke.mockRejectedValueOnce(new Error("磁盘已满"));
    await fireEvent.press(editor.getByTestId("dialog-save"));
    expect(await editor.findByTestId("dictionary-save-error")).toHaveTextContent("磁盘已满");
    await fireEvent.changeText(editor.getByTestId("dictionary-term"), "Voltip AI");
    expect(editor.queryByTestId("dictionary-save-error")).toBeNull();
    invoke.mockRejectedValueOnce(new Error("删不掉"));
    await fireEvent.press(editor.getByTestId("dictionary-delete"));
    await fireEvent.press(await screen.findByTestId("shell-confirm"));
    expect(await screen.findByText(/删不掉/)).toBeOnTheScreen();
    expect(screen.getByTestId("dictionary-editor")).toBeOnTheScreen();
    invoke.mockRestore();
    expect(backend.peek().dictionary.map((e) => [e.term, e.enabled])).toEqual([
      ["Voltip", true],
      ["飞书", true],
    ]);
  });
});
