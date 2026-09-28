import type { FeedbackKind, StagedAttachment } from "@voltip/shared";
import { useBackend } from "@voltip/ui";
import {
  type ReactNode,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";

/** What the 反馈 dialog is writing: kept above the dialog, so closing it (Esc, the scrim, ×) and
 *  opening it again finds the words and the staged files where they were, until they are sent or
 *  cleared (user decision 2026-09-28). */
export interface FeedbackDraftState {
  kind: FeedbackKind;
  message: string;
  contact: string;
  /** The files the shell staged for this draft, in order. */
  files: readonly StagedAttachment[];
}

export interface FeedbackDraftValue {
  draft: FeedbackDraftState;
  /** Merge `patch` into the draft; `files` may be a function of the current list. */
  update: (
    patch: Partial<Omit<FeedbackDraftState, "files">> & {
      files?:
        | readonly StagedAttachment[]
        | ((current: readonly StagedAttachment[]) => readonly StagedAttachment[]);
    },
  ) => void;
  /** Start over: empty words, and the staged files dropped in the shell too. */
  discard: () => void;
  /** After a report went out: empty the draft (the shell already forgot its files). */
  reset: () => void;
}

export const EMPTY_FEEDBACK_DRAFT: FeedbackDraftState = {
  kind: "bug",
  message: "",
  contact: "",
  files: [],
};

const FeedbackDraftContext = createContext<FeedbackDraftValue | undefined>(undefined);

/** Holds the 反馈 draft for the life of the window. The window starts with nothing staged: a
 *  reloaded webview cannot leave files in the shell that count against the limits. */
export function FeedbackDraftProvider({ children }: { children: ReactNode }) {
  const { backend } = useBackend();
  const [draft, setDraft] = useState<FeedbackDraftState>(EMPTY_FEEDBACK_DRAFT);

  useEffect(() => {
    void backend.feedbackAttachmentsClear().catch(() => undefined);
  }, [backend]);

  const update = useCallback<FeedbackDraftValue["update"]>((patch) => {
    setDraft((current) => {
      const { files, ...rest } = patch;
      const next = typeof files === "function" ? files(current.files) : files;
      return { ...current, ...rest, ...(next === undefined ? {} : { files: next }) };
    });
  }, []);
  const reset = useCallback(() => {
    setDraft(EMPTY_FEEDBACK_DRAFT);
  }, []);
  const discard = useCallback(() => {
    setDraft(EMPTY_FEEDBACK_DRAFT);
    void backend.feedbackAttachmentsClear().catch(() => undefined);
  }, [backend]);

  const value = useMemo(() => ({ draft, update, discard, reset }), [draft, update, discard, reset]);
  return <FeedbackDraftContext.Provider value={value}>{children}</FeedbackDraftContext.Provider>;
}

export function useFeedbackDraft(): FeedbackDraftValue {
  const value = useContext(FeedbackDraftContext);
  if (value === undefined) throw new Error("useFeedbackDraft needs a <FeedbackDraftProvider>");
  return value;
}
