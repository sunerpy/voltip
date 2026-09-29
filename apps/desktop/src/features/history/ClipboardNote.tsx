import type { HistoryOutcome } from "@voltip/shared";
import { Button, useBackend, useI18n, useUiState } from "@voltip/ui";
import { platformFromIdentity } from "../../app/platform";

export interface ClipboardNoteProps {
  outcome: Extract<HistoryOutcome, { kind: "clipboard" }>;
}

/** Under an entry whose paste fell back to the clipboard (docs/dictation.md §4.2): one sentence
 *  for the kind of reason, with the paste keys of this platform; the system setting when there is
 *  one to open (macOS Accessibility); the injector's own message only under the technical details.
 *  An entry written before the codes reads as `other`. */
export function ClipboardNote({ outcome }: ClipboardNoteProps) {
  const { t } = useI18n();
  const { backend } = useBackend();
  const state = useUiState();
  // The core's own OS: the paste keys and the setting belong to the machine that pasted.
  const platform = platformFromIdentity(state.identity?.platform);
  const code = outcome.code ?? "other";
  const keys = platform === "macos" ? "⌘V" : "Ctrl+V";
  return (
    <div
      className="flex flex-col gap-2 rounded-6 bg-warning-soft px-3 py-2 text-[12px] leading-5 text-warning"
      data-testid="history-clipboard-note"
      data-code={code}>
      <p>{t(`history.clipboardNote.${code}`, { keys })}</p>
      {code === "no_permission" && platform === "macos" && (
        <span>
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              void backend.permissionsRequest("accessibility");
            }}>
            {t("history.clipboardNote.openAccessibility")}
          </Button>
        </span>
      )}
      <details>
        <summary className="cursor-pointer text-fg-muted">
          {t("history.clipboardNote.details")}
        </summary>
        <p
          className="mono mt-1 break-all whitespace-pre-wrap text-fg-muted"
          data-testid="history-clipboard-detail"
          data-user-text>
          {outcome.reason}
        </p>
      </details>
    </div>
  );
}
