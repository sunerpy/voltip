import {
  type BatchAcceptOutcome,
  type BatchUndoOutcome,
  SUGGESTION_REPEATED_MIN,
  type Suggestion,
  type SuggestionPick,
  type SuggestionRefusal,
  type SuggestionUndo,
  type SuggestionsAnswer,
  type UndoOutcome,
  errorText,
} from "@voltip/shared";
import { useBackend, useUiState } from "@voltip/ui";
import { useEffect, useReducer, useRef } from "react";

/** A row's key: its pair, `heard` and `polished` exactly. */
export function suggestionKey(row: Pick<Suggestion, "heard" | "polished">): string {
  return JSON.stringify([row.heard, row.polished]);
}

/** Why 撤销 changed nothing. */
export type UndoFailure = Exclude<UndoOutcome, { kind: "undone" }>;

/** Why a batch's 撤销 changed nothing. */
export type BatchUndoFailure = Exclude<BatchUndoOutcome, { kind: "undone" }>;

/** Which suggestions the list shows: by default those seen `SUGGESTION_REPEATED_MIN` times or
 *  more, or all of them. */
export type SuggestionsFilter = "repeated" | "all";

/** What the user did to a row and how it went. A row acted on keeps its place while the page is
 *  open, even after the answer leaves it out (the dictionary corrects it now, or it is ignored). */
export type RowAction =
  | { kind: "busy"; what: "accept" | "ignore" }
  | { kind: "accepted"; undo: SuggestionUndo; busy: boolean }
  /** Accepted, and 撤销 did not happen. */
  | { kind: "kept"; failure: UndoFailure }
  | { kind: "ignored"; busy: boolean }
  /** Accepted or ignored in a batch: 撤销 is the batch's, over the list. */
  | { kind: "batched"; what: "accept" | "ignore"; batch: string }
  | { kind: "refused"; refusal: SuggestionRefusal }
  | { kind: "failed"; message: string };

/** What a batch did, shown over the list while the page is open: the rows it decided, how many it
 *  refused, and its 撤销; or why the batch did nothing. */
export type BatchNotice =
  | {
      id: string;
      kind: "accept" | "ignore";
      /** The rows the batch decided. */
      keys: readonly string[];
      /** Picks refused (加入词典 only); their reasons are under their rows. */
      refused: number;
      /** The core's id for 撤销; absent when nothing went in. */
      undo: string | undefined;
      undoing: boolean;
      /** Why 撤销 changed nothing. */
      failure: BatchUndoFailure | undefined;
    }
  | { id: string; kind: "failed"; message: string };

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
  /** The ignored pairs' keys as last seen, to notice one that 取消忽略 took off the list. */
  ignored: ReadonlySet<string> | undefined;
  filter: SuggestionsFilter;
  /** The checked rows' keys. */
  selected: ReadonlySet<string>;
  /** What the batches did, newest first. */
  batches: readonly BatchNotice[];
  /** A batch is on its way to the shell. */
  busy: boolean;
}

type Step =
  | { type: "answer"; answer: SuggestionsAnswer }
  | { type: "failed"; message: string }
  | { type: "act"; key: string; action: RowAction | undefined }
  | { type: "expand"; key: string | undefined }
  | { type: "ignored"; keys: ReadonlySet<string> }
  | { type: "filter"; filter: SuggestionsFilter }
  | { type: "select"; keys: readonly string[]; on: boolean }
  | { type: "batchStart"; keys: readonly string[]; what: "accept" | "ignore" }
  | { type: "batchAccepted"; id: string; keys: readonly string[]; outcome: BatchAcceptOutcome }
  | { type: "batchIgnored"; id: string; keys: readonly string[]; undo: string }
  | { type: "batchFailed"; id: string; keys: readonly string[]; message: string }
  | { type: "undoStart"; id: string }
  | { type: "undone"; id: string }
  | { type: "undoFailed"; id: string; failure: BatchUndoFailure };

const INITIAL: State = {
  load: { kind: "loading" },
  rows: [],
  actions: new Map(),
  expanded: undefined,
  ignored: undefined,
  filter: "repeated",
  selected: new Set(),
  batches: [],
  busy: false,
};

/** Whether a row passes `filter`. */
export function passes(row: Suggestion, filter: SuggestionsFilter): boolean {
  return filter === "all" || row.count >= SUGGESTION_REPEATED_MIN;
}

/** Whether a row with `action` is still to decide: never acted on, or the action did nothing. */
function undecided(action: RowAction | undefined): boolean {
  return action === undefined || action.kind === "refused" || action.kind === "failed";
}

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

/** `state.batches` with the notice `id` replaced by what `change` makes of it. */
function withBatch(
  state: State,
  id: string,
  change: (notice: Extract<BatchNotice, { kind: "accept" | "ignore" }>) => BatchNotice | undefined,
): readonly BatchNotice[] {
  return state.batches.flatMap((notice) => {
    if (notice.id !== id || notice.kind === "failed") return [notice];
    const next = change(notice);
    return next === undefined ? [] : [next];
  });
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
      // A row decided on its own is no longer checked.
      const selected = new Set(state.selected);
      if (!undecided(step.action)) selected.delete(step.key);
      return { ...state, actions, selected };
    }
    case "expand":
      return { ...state, expanded: step.key };
    case "ignored": {
      // A pair taken off the ignored list (取消忽略 in 已忽略) is no longer 已忽略 on its row either.
      const actions = new Map(state.actions);
      for (const key of state.ignored ?? []) {
        const action = actions.get(key);
        const ignored =
          action?.kind === "ignored" || (action?.kind === "batched" && action.what === "ignore");
        if (!step.keys.has(key) && ignored) actions.delete(key);
      }
      return { ...state, actions, ignored: step.keys };
    }
    case "filter": {
      // The rows the new view hides are no longer checked.
      const shown = new Set(
        state.rows.filter((r) => passes(r, step.filter)).map((r) => suggestionKey(r)),
      );
      const selected = new Set([...state.selected].filter((key) => shown.has(key)));
      return { ...state, filter: step.filter, selected };
    }
    case "select": {
      const selected = new Set(state.selected);
      for (const key of step.keys) {
        if (step.on) selected.add(key);
        else selected.delete(key);
      }
      return { ...state, selected };
    }
    case "batchStart": {
      const actions = new Map(state.actions);
      for (const key of step.keys) actions.set(key, { kind: "busy", what: step.what });
      return { ...state, actions, selected: new Set(), busy: true };
    }
    case "batchAccepted": {
      const actions = new Map(state.actions);
      const decided: string[] = [];
      let refused = 0;
      step.keys.forEach((key, i) => {
        const item = step.outcome.items[i];
        if (item?.kind === "accepted") {
          actions.set(key, { kind: "batched", what: "accept", batch: step.id });
          decided.push(key);
        } else if (item?.kind === "refused") {
          actions.set(key, { kind: "refused", refusal: item.refusal });
          refused += 1;
        } else {
          actions.delete(key);
        }
      });
      const notice: BatchNotice = {
        id: step.id,
        kind: "accept",
        keys: decided,
        refused,
        undo: step.outcome.undo ?? undefined,
        undoing: false,
        failure: undefined,
      };
      return { ...state, actions, batches: [notice, ...state.batches], busy: false };
    }
    case "batchIgnored": {
      const actions = new Map(state.actions);
      for (const key of step.keys) {
        actions.set(key, { kind: "batched", what: "ignore", batch: step.id });
      }
      const notice: BatchNotice = {
        id: step.id,
        kind: "ignore",
        keys: step.keys,
        refused: 0,
        undo: step.undo,
        undoing: false,
        failure: undefined,
      };
      return { ...state, actions, batches: [notice, ...state.batches], busy: false };
    }
    case "batchFailed": {
      const actions = new Map(state.actions);
      for (const key of step.keys) actions.delete(key);
      const notice: BatchNotice = { id: step.id, kind: "failed", message: step.message };
      return { ...state, actions, batches: [notice, ...state.batches], busy: false };
    }
    case "undoStart":
      return { ...state, batches: withBatch(state, step.id, (n) => ({ ...n, undoing: true })) };
    case "undone": {
      const actions = new Map(state.actions);
      for (const [key, action] of state.actions) {
        if (action.kind === "batched" && action.batch === step.id) actions.delete(key);
      }
      return { ...state, actions, batches: withBatch(state, step.id, () => undefined) };
    }
    case "undoFailed":
      return {
        ...state,
        batches: withBatch(state, step.id, (n) => ({
          ...n,
          undoing: false,
          failure: step.failure,
        })),
      };
  }
}

export interface SuggestionsView {
  load: SuggestionsLoad;
  /** Every row: the answer's and the ones acted on that it left out. */
  rows: readonly Suggestion[];
  /** The rows the filter shows. */
  visible: readonly Suggestion[];
  actions: ReadonlyMap<string, RowAction>;
  expanded: string | undefined;
  /** The suggestions still to decide that the filter shows (the 学习建议 tab's count), once the
   *  answer is in. */
  count: number | undefined;
  filter: SuggestionsFilter;
  setFilter: (filter: SuggestionsFilter) => void;
  /** The rows shown that can be checked: not decided, and not on their way to the shell. */
  selectable: readonly Suggestion[];
  /** The checked rows' keys, of `selectable`. */
  checked: ReadonlySet<string>;
  /** Check or clear `rows`. */
  select: (rows: readonly Suggestion[], on: boolean) => void;
  /** 加入词典 / 忽略 for the checked rows, in the list's order. */
  acceptChecked: () => void;
  ignoreChecked: () => void;
  /** 全部忽略: every row shown that is still to decide. */
  ignoreShown: () => void;
  /** What the batches did, newest first; a batch whose rows were all taken back is left out. */
  batches: readonly BatchNotice[];
  /** 撤销 of a batch. */
  undoBatch: (id: string) => void;
  /** A batch is on its way to the shell. */
  busy: boolean;
  accept: (row: Suggestion) => void;
  ignore: (row: Suggestion) => void;
  /** 撤销 of an accepted or ignored row. */
  undo: (row: Suggestion) => void;
  /** Open or close a row's details (one at a time). */
  toggle: (row: Suggestion) => void;
}

/** The dictionary's learning suggestions (`vocabulary_suggestions`, docs/dictation.md §16.10) and
 *  what the user did with them, one row or a batch at a time. The dictionary page holds it, so
 *  switching views keeps the rows, the filter, the checked rows and every 撤销; it ends with the
 *  page, as the undo records the shell keeps for it are not shown again (an ignored pair stays on
 *  已忽略, where 取消忽略 takes it back). Asked again whenever a list they are made from changes: the
 *  dictionary, the rules, the ignored pairs, the history or whether history is recorded. */
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
  const batchSeq = useRef(0);
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
  useEffect(() => {
    dispatch({ type: "ignored", keys: new Set(ignored.map(suggestionKey)) });
  }, [ignored]);

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

  const visible = state.rows.filter((r) => passes(r, state.filter));
  const selectable = visible.filter((r) => undecided(state.actions.get(suggestionKey(r))));
  const checked = new Set(
    selectable.map((r) => suggestionKey(r)).filter((key) => state.selected.has(key)),
  );
  const nextBatch = () => {
    batchSeq.current += 1;
    return `batch-${batchSeq.current}`;
  };
  const acceptRows = (rows: readonly Suggestion[]) => {
    const picked = rows.flatMap((row): [string, SuggestionPick][] => {
      const historyId = row.examples[0]?.history_id;
      return historyId === undefined
        ? []
        : [
            [
              suggestionKey(row),
              { heard: row.heard, polished: row.polished, history_id: historyId },
            ],
          ];
    });
    if (picked.length === 0 || state.busy) return;
    const id = nextBatch();
    const keys = picked.map(([key]) => key);
    dispatch({ type: "batchStart", keys, what: "accept" });
    backend.vocabularySuggestionsAccept(picked.map(([, pick]) => pick)).then(
      (outcome) => {
        dispatch({ type: "batchAccepted", id, keys, outcome });
      },
      (e: unknown) => {
        dispatch({ type: "batchFailed", id, keys, message: errorText(e) });
      },
    );
  };
  const ignoreRows = (rows: readonly Suggestion[]) => {
    if (rows.length === 0 || state.busy) return;
    const id = nextBatch();
    const keys = rows.map((row) => suggestionKey(row));
    dispatch({ type: "batchStart", keys, what: "ignore" });
    backend
      .vocabularySuggestionsIgnore(
        rows.map((row) => ({ heard: row.heard, polished: row.polished })),
      )
      .then(
        (outcome) => {
          dispatch(
            outcome.kind === "ignored"
              ? { type: "batchIgnored", id, keys, undo: outcome.undo }
              : { type: "batchFailed", id, keys, message: outcome.message },
          );
        },
        (e: unknown) => {
          dispatch({ type: "batchFailed", id, keys, message: errorText(e) });
        },
      );
  };
  const undoBatch = (id: string) => {
    const notice = state.batches.find((n) => n.id === id);
    if (notice === undefined || notice.kind === "failed") return;
    if (notice.undo === undefined || notice.undoing || notice.failure !== undefined) return;
    dispatch({ type: "undoStart", id });
    backend.vocabularySuggestionsUndo(notice.undo).then(
      (outcome) => {
        dispatch(
          outcome.kind === "undone"
            ? { type: "undone", id }
            : { type: "undoFailed", id, failure: outcome },
        );
      },
      (e: unknown) => {
        dispatch({ type: "undoFailed", id, failure: { kind: "failed", message: errorText(e) } });
      },
    );
  };
  // A batch whose rows were all taken back one by one (取消忽略 in 已忽略) has nothing left to show.
  const batches = state.batches.filter(
    (notice) =>
      notice.kind === "failed" ||
      notice.undo === undefined ||
      notice.keys.some((key) => {
        const action = state.actions.get(key);
        return action?.kind === "batched" && action.batch === notice.id;
      }),
  );

  const answer = state.load.kind === "answered" ? state.load.answer : undefined;
  return {
    ...state,
    visible,
    count:
      answer?.status === "ready"
        ? state.filter === "all"
          ? answer.total
          : answer.repeated
        : undefined,
    setFilter: (filter) => {
      dispatch({ type: "filter", filter });
    },
    selectable,
    checked,
    select: (rows, on) => {
      dispatch({ type: "select", keys: rows.map((row) => suggestionKey(row)), on });
    },
    acceptChecked: () => {
      acceptRows(selectable.filter((r) => checked.has(suggestionKey(r))));
    },
    ignoreChecked: () => {
      ignoreRows(selectable.filter((r) => checked.has(suggestionKey(r))));
    },
    ignoreShown: () => {
      ignoreRows(selectable);
    },
    batches,
    undoBatch,
    accept,
    ignore,
    undo,
    toggle,
  };
}
