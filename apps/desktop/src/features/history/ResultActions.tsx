import type { HistoryEntry } from "@voltip/shared";
import { IconButton, useBackend, useT } from "@voltip/ui";
import { useState } from "react";
import { copyWithToast, useShell } from "../../app/shell-context";
import { pasteWithToast } from "./paste";
import { textChars } from "./stats";

export interface ResultActionsProps {
  entry: HistoryEntry;
  className?: string;
}

/** The two buttons at the end of a result row (the home table, the history list): copy the
 *  result, or paste it into the window the user came from (`paste_text`). A voice edit's row
 *  hands on its rewrite (`entry.text`), like the history page's copy. A click stays on the button:
 *  it never opens the row. */
export function ResultActions({ entry, className }: ResultActionsProps) {
  const shell = useShell();
  const { backend } = useBackend();
  const t = useT();
  const [pasting, setPasting] = useState(false);
  const text = entry.text;
  const nothing = text.trim().length === 0;
  return (
    <span
      className={`inline-flex shrink-0 items-center gap-0.5 ${className ?? ""}`}
      data-testid="result-actions"
      onClick={(e) => {
        e.stopPropagation();
      }}>
      <IconButton
        icon="copy"
        label={t("paste.copy")}
        disabled={nothing}
        onClick={() => {
          void copyWithToast(shell, text, t("history.detail.copied", { n: textChars(text) }));
        }}
      />
      <IconButton
        icon="paste"
        label={t("paste.paste")}
        disabled={nothing || pasting}
        onClick={() => {
          setPasting(true);
          void pasteWithToast(shell, backend, text).finally(() => {
            setPasting(false);
          });
        }}
      />
    </span>
  );
}
