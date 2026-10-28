// The app's confirmation dialog (`shell.confirm`): 取消 declines unless a confirmation names its own
// button (手势粘贴's 暂不开启, docs/dictation.md §20.12), and the confirming button is red unless it
// confirms an agreement.
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react-native";
import { createTranslator } from "@voltip/shared";
import { useEffect } from "react";
import { StyleSheet, type TextStyle } from "react-native";
import { PaperProvider } from "react-native-paper";

import { I18nProvider } from "./backend/i18n";
import { type ConfirmSpec, ShellProvider, useShell } from "./shell";
import { appTheme } from "./theme/themes";

const theme = appTheme("light");

function Ask({ spec }: { spec: ConfirmSpec }) {
  const shell = useShell();
  useEffect(() => {
    shell.confirm(spec);
  }, [shell, spec]);
  return null;
}

async function ask(spec: ConfirmSpec) {
  await render(
    <I18nProvider locale="zh-CN">
      <PaperProvider theme={theme}>
        <ShellProvider bottomInset={0}>
          <Ask spec={spec} />
        </ShellProvider>
      </PaperProvider>
    </I18nProvider>,
  );
  return screen.findByTestId("shell-confirm");
}

/** The colour the confirming button's label `text` is drawn in. */
function labelColor(text: string): TextStyle["color"] {
  const label = within(screen.getByTestId("shell-confirm")).getByText(text);
  return StyleSheet.flatten(label.props.style as TextStyle).color;
}

describe("shell.confirm", () => {
  it("regression: declines with 取消 unless the confirmation names its own button", async () => {
    const onConfirm = jest.fn();
    await ask({ title: "删除词条", body: "删除后无法恢复。", confirmLabel: "删除", onConfirm });
    expect(screen.getByTestId("shell-confirm-cancel")).toHaveTextContent(
      createTranslator("zh-CN").t("mobile.cancel"),
    );
    await fireEvent.press(screen.getByTestId("shell-confirm-cancel"));
    await waitFor(() => {
      expect(screen.queryByText("删除词条")).toBeNull();
    });
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("regression: a confirmation's own decline button closes it without confirming", async () => {
    const onConfirm = jest.fn();
    await ask({
      title: "开启手势粘贴",
      body: "说明",
      confirmLabel: "同意并开启",
      cancelLabel: "暂不开启",
      tone: "primary",
      onConfirm,
    });
    expect(screen.getByTestId("shell-confirm-cancel")).toHaveTextContent("暂不开启");
    expect(screen.queryByText(createTranslator("zh-CN").t("mobile.cancel"))).toBeNull();
    await fireEvent.press(screen.getByTestId("shell-confirm-cancel"));
    await waitFor(() => {
      expect(screen.queryByText("开启手势粘贴")).toBeNull();
    });
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("colours the confirming button red for what cannot be undone, primary for an agreement", async () => {
    const onConfirm = jest.fn();
    const danger = await ask({ title: "删除", body: "b", confirmLabel: "删除", onConfirm });
    expect(labelColor("删除")).toBe(theme.colors.error);
    await fireEvent.press(danger);
    expect(onConfirm).toHaveBeenCalledTimes(1);
    await screen.unmount();
    await ask({
      title: "开启",
      body: "b",
      confirmLabel: "同意并开启",
      tone: "primary",
      onConfirm,
    });
    expect(labelColor("同意并开启")).toBe(theme.colors.primary);
  });
});
