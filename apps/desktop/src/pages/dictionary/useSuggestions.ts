import {
  type Suggestion,
  type SuggestionRefusal,
  type SuggestionUndo,
  type SuggestionsAnswer,
  type UndoOutcome,
  errorText,
} from "@voltip/shared";
import { useBackend, useUiState } from "@voltip/ui";
import { useEffect, useReducer } from "react";

/** A row's key: its pair, `heard` and `polished` exactly. */
export function suggestionKey(row: Pick<Suggestion, "heard" | "polished">): string {
  return JSON.stringify([row.heard, row.polished]);
}

/** Why 撤销 changed nothing. */
export type UndoFailure = Exclude<UndoOutcome, { kind: "undone" }>;

/** What the user did to a row and how it went. A row acted on keeps its place while the page is
 *  open, even after the answer leaves it out (the dictionary corrects it now, or it is ignored). */
export type RowAction =
  | { kind: "busy"; what: "accept" | "ignore" }
  | { kind: "accepted"; undo: SuggestionUndo; busy: boolean }
  /** Accepted, and 撤销 did not happen. */
  | { kind: "kept"; failure: UndoFailure }
  | { kind: "ignored"; busy: boolean }
  | { kind: "refused"; refusal: SuggestionRefusal }
  | { kind: "failed"; message: string };

/** Where the shell's answer is: being asked, refused, or in. */
export type SuggestionsLoad =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | { kind: "answered"; answer: SuggestionsAnswer };

interface State {
  load: SuggestionsLoad;
  rows: readonly Suggestion[];
  actions: ReadonlyMap<string, RowAction>;
  /** The row whose details are open. */
  expanded: string | undefined;
}

type Step =
  | { type: "answer"; answer: SuggestionsAnswer }
  | { type: "failed"; message: string }
  | { type: "act"; key: string; action: RowAction | undefined }
  | { type: "expand"; key: string | undefined };

const INITIAL: State = {
  load: { kind: "loading" },
  rows: [],
  actions: new Map(),
  expanded: undefined,
};

/** The answer's rows, with each row acted on that it no longer has back at its place. */
export function mergeRows(
  shown: readonly Suggestion[],
  next: readonly Suggestion[],
  actions: ReadonlyMap<string, RowAction>,
): Suggestion[] {
  const out = [...next];
  const live = new Set(next.map(suggestionKey));
  shown.forEach((row, index) => {
    const key = suggestionKey(row);
    if (actions.has(key) && !live.has(key)) out.splice(Math.min(index, out.length), 0, row);
  });
  return out;
}

function reduce(state: State, step: Step): State {
  switch (step.type) {
    case "answer":
      return {
        ...state,
        load: { kind: "answered", answer: step.answer },
        rows:
          step.answer.status === "ready"
            ? mergeRows(state.rows, step.answer.pairs, state.actions)
            : [],
      };
    case "failed":
      return { ...state, load: { kind: "failed", message: step.message } };
    case "act": {
      const actions = new Map(state.actions);
      if (step.action === undefined) actions.delete(step.key);
      else actions.set(step.key, step.action);
      return { ...state, actions };
    }
    case "expand":
      return { ...state, expanded: step.key };
  }
}

export interface SuggestionsView {
  load: SuggestionsLoad;
  rows: readonly Suggestion[];
  actions: ReadonlyMap<string, RowAction>;
  expanded: string | undefined;
  /** The suggestions still to decide (the 学习建议 tab's count), once the answer is in. */
  count: number | undefined;
  accept: (row: Suggestion) => void;
  ignore: (row: Suggestion) => void;
  /** 撤销 of an accepted or ignored row. */
  undo: (row: Suggestion) => void;
  /** Open or close a row's details (one at a time). */
  toggle: (row: Suggestion) => void;
}

/** The dictionary's learning suggestions (`vocabulary_suggestions`, docs/dictation.md §16.10) and
 *  what the user did with them. The dictionary page holds it, so switching views keeps the rows
 *  and their 撤销; it ends with the page, as the undo records the shell keeps for it are not shown
 *  again. Asked again whenever a list they are made from changes: the dictionary, the rules, the
 *  ignored pairs, the history or whether history is recorded. */
export function useSuggestions(): SuggestionsView {
  const { backend } = useBackend();
  const {
    dictionary,
    rules,
    vocabulary_ignored: ignored,
    history_recent: recent,
    history_total: total,
    settings,
  } = useUiState();
  const recording = settings.history.enabled;
  const [state, dispatch] = useReducer(reduce, INITIAL);
  useEffect(() => {
    let alive = true;
    backend.vocabularySuggestions().then(
      (answer) => {
        if (alive) dispatch({ type: "answer", answer });
      },
      (e: unknown) => {
        if (alive) dispatch({ type: "failed", message: errorText(e) });
      },
    );
    return () => {
      alive = false;
    };
    // Each list the suggestions are made from is a reason to ask again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [backend, dictionary, rules, ignored, recent, total, recording]);

  const act = (key: string, action: RowAction | undefined) => {
    dispatch({ type: "act", key, action });
  };
  const failed = (key: string) => (e: unknown) => {
    act(key, { kind: "failed", message: errorText(e) });
  };
  const accept = (row: Suggestion) => {
    // A pair always comes from a take; the first one it was seen in is the new entry's source.
    const historyId = row.examples[0]?.history_id;
    if (historyId === undefined) return;
    const key = suggestionKey(row);
    act(key, { kind: "busy", what: "accept" });
    backend
      .vocabularySuggestionAccept({ heard: row.heard, polished: row.polished, historyId })
      .then((outcome) => {
        act(
          key,
          outcome.kind === "accepted"
            ? { kind: "accepted", undo: outcome.undo, busy: false }
            : { kind: "refused", refusal: outcome.refusal },
        );
      }, failed(key));
  };
  const ignore = (row: Suggestion) => {
    const key = suggestionKey(row);
    act(key, { kind: "busy", what: "ignore" });
    backend
      .invoke("vocabulary_suggestion_ignore", { heard: row.heard, polished: row.polished })
      .then(() => {
        act(key, { kind: "ignored", busy: false });
      }, failed(key));
  };
  const undo = (row: Suggestion) => {
    const key = suggestionKey(row);
    const action = state.actions.get(key);
    if (action?.kind === "accepted" && !action.busy) {
      act(key, { ...action, busy: true });
      backend.vocabularySuggestionUndo(action.undo.id).then(
        (outcome) => {
          act(key, outcome.kind === "undone" ? undefined : { kind: "kept", failure: outcome });
        },
        (e: unknown) => {
          act(key, { kind: "kept", failure: { kind: "failed", message: errorText(e) } });
        },
      );
    } else if (action?.kind === "ignored" && !action.busy) {
      act(key, { kind: "ignored", busy: true });
      backend
        .invoke("vocabulary_suggestion_unignore", { heard: row.heard, polished: row.polished })
        .then(() => {
          act(key, undefined);
        }, failed(key));
    }
  };
  const toggle = (row: Suggestion) => {
    const key = suggestionKey(row);
    dispatch({ type: "expand", key: state.expanded === key ? undefined : key });
  };
  const answer = state.load.kind === "answered" ? state.load.answer : undefined;
  return {
    ...state,
    count: answer?.status === "ready" ? answer.total : undefined,
    accept,
    ignore,
    undo,
    toggle,
  };
}
