import type { Backend, PasteOutcome, TFunction } from "@voltip/shared";
import type { ToastInput } from "@voltip/ui";
import type { ShellActions } from "../../app/shell-context";

const PASTED_TOAST_MS = 2000;
const NOT_PASTED_TOAST_MS = 4000;

/** The toast a paste from the history ends with: done, copied instead (and why), or refused. */
export function pasteToast(outcome: PasteOutcome, t: TFunction): ToastInput {
  switch (outcome.kind) {
    case "pasted":
      return { message: t("paste.outcome.pasted"), duration: PASTED_TOAST_MS };
    case "copied":
      return {
        message: t(`paste.outcome.copied.${outcome.reason}`),
        duration: NOT_PASTED_TOAST_MS,
      };
    case "failed":
      return {
        message: t(`paste.outcome.failed.${outcome.reason}`),
        duration: NOT_PASTED_TOAST_MS,
        tone: "danger",
      };
  }
}

/** 「粘贴到上一个窗口」: hand `text` to the shell (`paste_text`) and say what became of it. A
 *  command that fails outright reads like one that did not finish. */
export async function pasteWithToast(
  shell: Pick<ShellActions, "toast" | "t">,
  backend: Pick<Backend, "pasteText">,
  text: string,
): Promise<PasteOutcome> {
  let outcome: PasteOutcome;
  try {
    outcome = await backend.pasteText(text);
  } catch (_error) {
    outcome = { kind: "failed", reason: "timeout" };
  }
  shell.toast(pasteToast(outcome, shell.t));
  return outcome;
}
