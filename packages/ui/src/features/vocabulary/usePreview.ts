import { type PreviewDraft, type VocabularyPreview, errorText } from "@voltip/shared";
import { useEffect, useState } from "react";
import { useBackend, useUiState } from "../../backend/BackendProvider";

/** Quiet time after the last keystroke before the core is asked again. */
export const PREVIEW_DEBOUNCE_MS = 200;

export type PreviewState =
  | { kind: "idle" }
  | { kind: "ok"; preview: VocabularyPreview }
  | { kind: "error"; message: string };

const IDLE: PreviewState = { kind: "idle" };

/** `text` through the core's dictionary and rules (`vocabulary_preview`), with `draft` standing in
 *  for the rule it names; re-asked when the text, the draft or either list changes. An empty text
 *  is only sent when `allowEmpty` (the rule editor uses it to have the core check a draft). Pass a
 *  memoised `draft`: a new object every render asks again every render. */
export function useVocabularyPreview(
  text: string,
  draft?: PreviewDraft,
  allowEmpty = false,
): PreviewState {
  const { backend } = useBackend();
  const { dictionary, rules } = useUiState();
  const [state, setState] = useState<PreviewState>(IDLE);
  const active = allowEmpty || text.length > 0;
  useEffect(() => {
    if (!active) return;
    let alive = true;
    const handle = setTimeout(() => {
      backend.vocabularyPreview(text, draft).then(
        (preview) => {
          if (alive) setState({ kind: "ok", preview });
        },
        (e: unknown) => {
          if (alive) setState({ kind: "error", message: errorText(e) });
        },
      );
    }, PREVIEW_DEBOUNCE_MS);
    return () => {
      alive = false;
      clearTimeout(handle);
    };
    // The core previews against the lists it caches: a changed list is a reason to ask again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [backend, text, draft, active, dictionary, rules]);
  return active ? state : IDLE;
}
