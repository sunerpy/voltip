// The history page's detail (moved out of `History.tsx`, which keeps the list) with the quick fix
// of docs/dictation.md §16.9: marks on the original text, a toolbar over selected words, a card
// that writes a dictionary entry or a rule, the diff view's replacements, and a toast that undoes
// the save (Ctrl Z). Only the takes this computer recognised get the tools (§16.9 3.8).
import {
  type FixKind,
  type FixTarget,
  type FixView,
  type HistoryEntry,
  type PolishPair,
  type TextMark,
  diffItems,
  diffSegments,
  errorText,
  fixScope,
  fixTarget,
  formatTakeLength,
  isBuiltinPreset,
  outcomeDetailText,
  prefillFor,
  presetRefLabel,
  recordingOfEntry,
  refineModelText,
  sceneLabel,
  takePolishPairs,
  textMarks,
  wordAt,
} from "@voltip/shared";
import {
  Badge,
  Button,
  IconButton,
  Keycaps,
  Readout,
  Segmented,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { inTextField, usePageShortcuts, withCommand } from "../../app/page-shortcuts";
import { useRouter } from "../../app/router";
import { useShell } from "../../app/shell-context";
import { ClipboardNote } from "../../features/history/ClipboardNote";
import { useRetake } from "../../features/history/Recordings";
import { RetakeModelPickers, useRetakeModels } from "../../features/history/RetakeModels";
import { RefineNote } from "../../features/history/RefineNote";
import {
  LongEntryTools,
  isLongEntry,
  useHistoryProcess,
} from "../../features/history/LongEntryTools";
import { textChars } from "../../features/history/stats";
import { EditDetail, TimingBar, VocabularyHits } from "./DetailParts";
import { DiffText, MarkedText } from "./FixText";
import { type CardOpen, KnownCard, QuickFixCard, type SavedFix } from "./QuickFixCard";
import { SelectionToolbar } from "./SelectionToolbar";
import { type FloatBox, boxOf, useFloatingStyle } from "./floating";
import { boundaryOffset, selectOffsets, selectionIn } from "./text-offsets";

export type View = "raw" | "polished" | "diff" | "processed";

/** How long the saved toast offers 撤销 (§16.9 3.6). */
export const UNDO_MS = 5000;

/** A phone's text rather than a recognised take (docs/dictation.md §20.6): no model, no timings. */
export function sentAsText(entry: HistoryEntry): boolean {
  return entry.origin?.kind === "typed" || entry.origin?.kind === "clipboard";
}

/** The text the user selected inside this entry's raw or final text, as 加入词典 pre-fills it
 *  when no selection of the text box is being tracked (the keyboard's way, a voice edit). */
function selectedFragment(entry: HistoryEntry): string {
  const text = window.getSelection()?.toString().trim() ?? "";
  if (text.length === 0 || text.includes("\n")) return "";
  return entry.raw_text.includes(text) || entry.text.includes(text) ? text : "";
}

/** Where a double click lands in the text (the caret under the pointer, else the selection the
 *  webview made). */
function clickOffset(root: Element, x: number, y: number, length: number): number | undefined {
  if ("caretPositionFromPoint" in document) {
    const pos = document.caretPositionFromPoint(x, y);
    if (pos !== null) return boundaryOffset(root, pos.offsetNode, pos.offset, length);
  }
  if ("caretRangeFromPoint" in document) {
    const range = document.caretRangeFromPoint(x, y);
    if (range !== null)
      return boundaryOffset(root, range.startContainer, range.startOffset, length);
  }
  const selection = window.getSelection();
  if (
    selection === null ||
    typeof selection.getRangeAt !== "function" ||
    selection.rangeCount === 0
  ) {
    return undefined;
  }
  const range = selection.getRangeAt(0);
  return boundaryOffset(root, range.startContainer, range.startOffset, length);
}

/** The selection being tracked: the entry it is in, what it became for the card, and where it is. */
interface LiveSelection {
  entryId: string;
  target: FixTarget;
  rect: FloatBox;
}

/** An open card: what it opened with, what it is anchored to, where the focus goes back. */
interface CardState {
  entryId: string;
  open: CardOpen | { mode: "known"; mark: TextMark };
  anchorEl?: HTMLElement;
  rect?: FloatBox;
  back: HTMLElement | null;
  /** The words drawn as selected while the card is open. */
  highlight?: { view: FixView; start: number; end: number };
  /** The diff view's replacement, ringed while its card is open. */
  pair?: PolishPair;
  seq: number;
}

export interface HistoryDetailProps {
  entry: HistoryEntry;
  view: View;
  onView: (view: View) => void;
  onCopy: () => void;
  onStar: () => void;
  onRemove: () => void;
  starLabel: string;
}

export function HistoryDetail({
  entry,
  view,
  onView,
  onCopy,
  onStar,
  onRemove,
  starLabel,
}: HistoryDetailProps) {
  const shell = useShell();
  const { navigate } = useRouter();
  const { t, locale } = useI18n();
  const state = useUiState();
  // docs/dictation.md §22: 用 AI 预设处理 on the entry in the detail, and its result.
  const process = useHistoryProcess(entry.id);
  const processed = process.view.state === "done" ? process.view.processed : entry.processed;
  const shown: View = view === "processed" && processed === undefined ? "polished" : view;
  const scope = fixScope(entry, "desktop");
  const fixing = scope.kind === "fix";
  // docs/dictation.md §26: the recording this entry was made from, while the cache keeps it.
  const kept = recordingOfEntry(state.audio_cache, entry.id);
  const retakes = useRetake();
  const retakeModels = useRetakeModels();
  // 重新润色 (§26): 用 AI 预设处理 with the preset the take would have used — its scene's, else the
  // engines' — kept as the processed text, which the detail then shows.
  const scenePreset = state.scenes.find((s) => s.id === entry.scene?.id)?.overrides.refine_preset;
  const retryPreset = scenePreset ?? state.settings.engines.refine_preset;
  const refineRetry = useRef(false);
  const processView = process.view;
  useEffect(() => {
    if (!refineRetry.current) return;
    if (processView.state === "done") {
      refineRetry.current = false;
      onView("processed");
      shell.toast({ message: t("history.refineNote.retried"), duration: 4000 });
    } else if (processView.state === "failed") {
      refineRetry.current = false;
      shell.toast({
        message: t("history.long.failed", { reason: processView.reason }),
        duration: 6000,
        tone: "danger",
      });
    } else if (processView.state === "cancelled") {
      refineRetry.current = false;
    }
  }, [processView, onView, shell, t]);

  // The replacements the AI polish made: none when the take was not polished, and none that its
  // own corrections and rules account for (§16.9 3.1).
  const pairs = useMemo(
    () => (fixing ? takePolishPairs(entry, state.dictionary, state.rules) : []),
    [fixing, entry, state.dictionary, state.rules],
  );
  const marks = useMemo(
    () => (fixing && shown === "raw" ? textMarks(entry.raw_text, pairs, state.dictionary) : []),
    [fixing, shown, entry.raw_text, pairs, state.dictionary],
  );
  const items = useMemo(
    () => (fixing && shown === "diff" ? diffItems(entry.raw_text, entry.text, pairs) : []),
    [fixing, shown, entry.raw_text, entry.text, pairs],
  );

  const box = useRef<HTMLDivElement>(null);
  const textRoot = useRef<HTMLSpanElement>(null);
  // The selection, the card and the tooltip belong to an entry (and the selection to a view):
  // another one shown, they are not there any more.
  const [liveState, setLive] = useState<LiveSelection | undefined>(undefined);
  const [cardState, setCard] = useState<CardState | undefined>(undefined);
  const [tipState, setTip] = useState<
    { entryId: string; mark: TextMark; el: HTMLElement } | undefined
  >(undefined);
  const live =
    liveState?.entryId === entry.id && liveState.target.view === shown ? liveState : undefined;
  const card = cardState?.entryId === entry.id ? cardState : undefined;
  const tip = tipState?.entryId === entry.id && card === undefined ? tipState : undefined;
  const dirty = useRef(false);
  const seq = useRef(0);
  const latest = useRef(state);
  const cardNow = useRef(card);
  useEffect(() => {
    latest.current = state;
    cardNow.current = card;
  });
  const undo = useRef<{ run: () => void; until: number; toast: string } | undefined>(undefined);

  const clearSelection = useCallback(() => {
    window.getSelection()?.removeAllRanges();
    setLive(undefined);
  }, []);

  const readSelection = useCallback(
    (fixView: FixView) => {
      const root = textRoot.current;
      const source = fixView === "raw" ? entry.raw_text : entry.text;
      if (root === null) return;
      const selected = selectionIn(root, source.length);
      const target =
        selected === undefined
          ? undefined
          : fixTarget(entry, fixView, selected.start, selected.end, pairs);
      if (selected === undefined || target === undefined) {
        setLive(undefined);
        return;
      }
      // Show what the card will act on: the trimmed words, grown to the whole replacement.
      if (target.start !== selected.start || target.end !== selected.end) {
        selectOffsets(root, target.start, target.end);
      }
      setLive({ entryId: entry.id, target, rect: selected.rect });
    },
    [entry, pairs],
  );

  // The toolbar goes when the selection is gone, leaves the text, or the page scrolls.
  useEffect(() => {
    if (live === undefined) return undefined;
    const onChange = () => {
      const root = textRoot.current;
      const selection = window.getSelection();
      const gone =
        root === null ||
        selection === null ||
        selection.rangeCount === 0 ||
        selection.isCollapsed ||
        !selection.getRangeAt(0).intersectsNode(root);
      if (gone) setLive(undefined);
    };
    const onScroll = () => {
      setLive(undefined);
    };
    document.addEventListener("selectionchange", onChange);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      document.removeEventListener("selectionchange", onChange);
      window.removeEventListener("scroll", onScroll, true);
    };
  }, [live]);

  const openCard = useCallback(
    (next: Omit<CardState, "seq" | "entryId">) => {
      // A card with typed words stays until 取消 or Esc (§16.9 3.3): another does not replace it.
      if (cardNow.current !== undefined && dirty.current) return;
      dirty.current = false;
      seq.current += 1;
      setTip(undefined);
      setCard({ ...next, entryId: entry.id, seq: seq.current });
    },
    [entry.id],
  );

  const openFromSelection = useCallback(
    (selection: LiveSelection, kind: FixKind, anchorEl?: HTMLElement) => {
      const { target } = selection;
      openCard({
        open: {
          mode: "fix",
          kind,
          prefill: { ...prefillFor(target), kind },
          polishOnly: target.polishOnly,
        },
        ...(anchorEl === undefined ? {} : { anchorEl }),
        rect: selection.rect,
        back: anchorEl ?? box.current,
        highlight: { view: target.view, start: target.start, end: target.end },
      });
      // The native selection goes with the focus; the card's words are drawn as selected.
      clearSelection();
    },
    [openCard, clearSelection],
  );

  const openFromMark = useCallback(
    (mark: TextMark, el: HTMLElement) => {
      if (mark.kind === "known") {
        openCard({ open: { mode: "known", mark }, anchorEl: el, back: el });
        return;
      }
      const polished = mark.pair?.polished ?? "";
      openCard({
        open: {
          mode: "fix",
          kind: "dictionary",
          prefill: { kind: "dictionary", heard: mark.text, term: polished, termFrom: "polish" },
          polishOnly: false,
        },
        anchorEl: el,
        back: el,
        highlight: { view: "raw", start: mark.start, end: mark.end },
      });
    },
    [openCard],
  );

  const openFromPair = useCallback(
    (pair: PolishPair, el: HTMLElement) => {
      openCard({ open: { mode: "which", pair }, anchorEl: el, back: el, pair });
    },
    [openCard],
  );

  /** The detail's own 加入词典 (§16.9 3.2): the same card, next to the button. */
  const openFromButton = (el: HTMLElement) => {
    if (live !== undefined) {
      openFromSelection(live, "dictionary", el);
      return;
    }
    openCard({
      open: {
        mode: "fix",
        kind: "dictionary",
        prefill: { kind: "dictionary", heard: selectedFragment(entry), term: "" },
        polishOnly: false,
      },
      anchorEl: el,
      back: el,
    });
  };

  const closeCard = useCallback((refocus: boolean) => {
    const was = cardNow.current;
    setCard(undefined);
    dirty.current = false;
    // Esc gives the focus back to the mark or the text the card came from (§16.9 3.7).
    if (refocus && was !== undefined) {
      (was.back?.isConnected === true ? was.back : box.current)?.focus();
    }
  }, []);

  const onSaved = useCallback(
    (saved: SavedFix) => {
      closeCard(true);
      clearSelection();
      const run = () => {
        undo.current = undefined;
        saved.undo(latest.current).catch((e: unknown) => {
          shell.toast({ message: errorText(e), tone: "danger", duration: 3000 });
        });
      };
      const toast = shell.toast({
        message: saved.message,
        duration: UNDO_MS,
        action: { label: t("history.fix.undo"), keys: "Ctrl Z", onClick: run },
      });
      undo.current = { run, until: Date.now() + UNDO_MS, toast };
    },
    [closeCard, clearSelection, shell, t],
  );

  const openDictionary = useCallback(() => {
    closeCard(false);
    navigate({ name: "dictionary" });
  }, [closeCard, navigate]);

  // A click outside a card nobody typed in closes it (§16.9 3.3).
  useEffect(() => {
    if (card === undefined) return undefined;
    const onDown = (e: PointerEvent) => {
      const panel = document.querySelector("[data-fix-panel]");
      if (e.target instanceof Node && panel?.contains(e.target) === true) return;
      if (!dirty.current) closeCard(false);
    };
    document.addEventListener("pointerdown", onDown, true);
    return () => {
      document.removeEventListener("pointerdown", onDown, true);
    };
  }, [card, closeCard]);

  // D / R on a selection, Esc to drop it; Esc closes a card the focus left; Ctrl Z undoes the
  // last save while its toast is up (§16.9 3.6, 3.7).
  usePageShortcuts((e) => {
    if (withCommand(e) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "z") {
      const pending = undo.current;
      if (pending === undefined || Date.now() > pending.until || inTextField(e.target))
        return false;
      shell.toasts.dismiss(pending.toast);
      pending.run();
      return true;
    }
    if (card !== undefined && e.key === "Escape") {
      closeCard(true);
      return true;
    }
    if (
      live === undefined ||
      card !== undefined ||
      withCommand(e) ||
      e.altKey ||
      inTextField(e.target)
    ) {
      return false;
    }
    const key = e.key.toLowerCase();
    if (key === "escape") {
      clearSelection();
      return true;
    }
    if (key === "d" && live.target.block === undefined) {
      openFromSelection(live, "dictionary");
      return true;
    }
    if (key === "r") {
      openFromSelection(live, "rule");
      return true;
    }
    return false;
  });

  const cardAnchor = useCallback((): FloatBox | undefined => {
    if (card === undefined) return undefined;
    return (
      boxOf(card.anchorEl) ??
      boxOf(textRoot.current?.querySelector("[data-fix-highlight]")) ??
      card.rect
    );
  }, [card]);

  const markLabel = (mark: TextMark) =>
    mark.kind === "known"
      ? t("history.fix.marks.knownLabel", { heard: mark.text, term: mark.entry?.term ?? "" })
      : t("history.fix.marks.polishedLabel", {
          heard: mark.text,
          polished: mark.pair?.polished ?? "",
        });
  const pairLabel = (pair: PolishPair) =>
    t("history.fix.diff.pairLabel", { del: pair.heard, add: pair.polished });
  const onHint = (mark: TextMark | undefined, el: HTMLElement) => {
    setTip(mark === undefined || card !== undefined ? undefined : { entryId: entry.id, mark, el });
  };
  const highlight = card?.highlight?.view === shown ? card.highlight : undefined;
  const polishCount = marks.filter((m) => m.kind === "polish").length;
  const knownCount = marks.length - polishCount;

  /** The text of a view the quick fix works on, as leaves the selection maps back to. */
  const fixText = (fixView: FixView, source: string) => (
    <span
      ref={textRoot}
      data-fix-text={fixView}
      onMouseUp={() => {
        readSelection(fixView);
      }}
      onDoubleClick={(e) => {
        // The same words on every platform: the segmenter's, not the webview's (§16.9 3.2).
        const root = textRoot.current;
        if (root === null) return;
        const at = clickOffset(root, e.clientX, e.clientY, source.length);
        const word = at === undefined ? undefined : wordAt(source, at);
        if (word !== undefined) selectOffsets(root, word.start, word.end);
        readSelection(fixView);
      }}>
      <MarkedText
        text={source}
        marks={fixView === "raw" ? marks : []}
        highlight={highlight}
        label={markLabel}
        onMark={openFromMark}
        onHint={onHint}
      />
    </span>
  );

  return (
    <div className="flex h-full flex-col gap-4">
      {entry.kind === "edit" && entry.edit !== undefined ? (
        <EditDetail entry={entry} edit={entry.edit} t={t} />
      ) : (
        <>
          <div className="flex items-center justify-between">
            <Segmented
              label={t("history.view.label")}
              size="sm"
              value={shown}
              onChange={onView}
              options={[
                { value: "raw", label: t("history.view.raw") },
                {
                  value: "polished",
                  label: entry.refined ? t("history.view.polished") : t("history.view.inserted"),
                },
                { value: "diff", label: t("history.view.diff") },
                ...(processed === undefined
                  ? []
                  : [{ value: "processed" as const, label: t("history.view.processed") }]),
              ]}
            />
            {shown === "diff" && (
              <span className="flex gap-2">
                <Badge tone="danger">{t("history.view.deleted")}</Badge>
                <Badge tone="accent">{t("history.view.added")}</Badge>
              </span>
            )}
          </div>
          <div
            ref={box}
            tabIndex={-1}
            className="min-h-[120px] rounded-10 bg-inset p-4 text-[15px] leading-7 text-fg outline-none"
            data-testid="entry-text"
            data-user-text>
            {shown === "raw" && (fixing ? fixText("raw", entry.raw_text) : entry.raw_text)}
            {shown === "polished" && (fixing ? fixText("polished", entry.text) : entry.text)}
            {shown === "processed" && processed !== undefined && (
              <span className="whitespace-pre-wrap" data-testid="entry-processed">
                {processed.text}
              </span>
            )}
            {shown === "diff" &&
              (fixing ? (
                <DiffText
                  items={items}
                  active={card?.pair}
                  label={pairLabel}
                  onPair={openFromPair}
                />
              ) : (
                diffSegments({ rawText: entry.raw_text, text: entry.text }).map((seg, i) =>
                  seg.kind === "same" ? (
                    <span key={i}>{seg.text}</span>
                  ) : seg.kind === "del" ? (
                    <span
                      key={i}
                      className="mx-0.5 rounded-4 bg-diff-del px-1 text-danger line-through">
                      {seg.text}
                    </span>
                  ) : (
                    <span key={i} className="mx-0.5 rounded-4 bg-diff-add px-1 text-ok-text">
                      {seg.text}
                    </span>
                  ),
                )
              ))}
            {shown === "raw" && marks.length > 0 && (
              <MarkLegend polish={polishCount} known={knownCount} />
            )}
            {shown === "diff" && items.some((i) => i.kind === "pair") && (
              <div className="mt-3 text-[12px] text-fg-subtle" data-testid="fix-diff-hint">
                {t("history.fix.diff.hint")}
              </div>
            )}
            {shown === "processed" && processed !== undefined ? (
              <div className="mt-3 text-[12px] text-fg-subtle">
                {t("history.view.processedBy", {
                  preset: presetRefLabel(processed.preset, locale),
                })}
              </div>
            ) : (
              shown !== "raw" && (
                <div className="mt-3 text-[12px] text-fg-subtle">
                  {t("history.view.rawOutput", { text: entry.raw_text })}
                </div>
              )
            )}
          </div>
          {scope.kind === "note" && (
            <p
              className="flex items-start gap-2.5 rounded-10 bg-inset px-3 py-2 text-[12px] leading-[18px] text-fg"
              data-testid="fix-not-here">
              {t(`history.fix.notHere.${scope.note}`)}
            </p>
          )}
        </>
      )}
      {isLongEntry(entry) && <LongEntryTools entry={entry} process={process} />}
      {entry.outcome.kind === "clipboard" && (
        // docs/dictation.md §4.2: why the paste fell back, in words; the raw message only under the
        // technical details (never in the header).
        <ClipboardNote outcome={entry.outcome} />
      )}
      {!entry.refined && entry.refine_error !== undefined && (
        // §3.6: the clean-up's request failed; why, in words, and the service's own explanation
        // under the technical details. §26: 重新润色 when an AI service is set up.
        <RefineNote
          entry={entry}
          retrying={processView.state === "running"}
          onRetry={
            state.engines.refine_ready && !isLongEntry(entry)
              ? () => {
                  refineRetry.current = true;
                  process.start(retryPreset);
                }
              : undefined
          }
        />
      )}
      {kept !== undefined && (
        // docs/dictation.md §26: the recording is still kept: 重新识别 makes a new entry from it.
        <div
          className="flex flex-wrap items-center gap-x-3 gap-y-2 rounded-10 hairline px-3 py-2"
          data-testid="history-kept-recording">
          <div className="min-w-0 flex-1">
            <p className="text-[12px] text-fg">{t("history.recordings.detail")}</p>
            <p className="text-[11px] leading-4 text-fg-subtle">
              {t("history.recordings.detailHelp")}
            </p>
          </div>
          <div className="flex w-full flex-wrap items-center justify-between gap-2">
            <RetakeModelPickers choice={retakeModels} disabled={retakes.busy} />
            <Button
              size="sm"
              icon="refresh"
              disabled={retakes.busy}
              data-testid="history-detail-retake"
              onClick={() => {
                retakes.retake(kept.id, retakeModels.models);
              }}>
              {retakes.running === kept.id
                ? t("history.recordings.retaking")
                : t("history.recordings.retake")}
            </Button>
          </div>
        </div>
      )}
      {entry.live_error !== undefined && (
        // §12: a streaming mode was asked for but the take (or part of it) fell back to the
        // whole-take path; the core's reason, verbatim.
        <p
          className="rounded-6 bg-warning-soft px-3 py-2 text-[12px] leading-4 text-warning"
          data-testid="history-live-error">
          {t("history.liveError", { reason: entry.live_error })}
        </p>
      )}
      {entry.vocabulary !== undefined && <VocabularyHits hits={entry.vocabulary} t={t} />}
      {!sentAsText(entry) && <TimingBar entry={entry} t={t} />}
      <div className="grid grid-cols-2 gap-x-6 gap-y-2 border-t border-border pt-3">
        <Readout label={t("history.detail.asrModel")} value={entry.asr_model} size="sm" />
        <Readout
          label={t("history.detail.refineModel")}
          value={refineModelText(entry, locale)}
          size="sm"
          muted={!entry.refined}
        />
        {entry.preset !== undefined && (
          // docs/dictation.md §21: the preset the clean-up ran with, by the name it had.
          <Readout
            label={t("history.detail.preset")}
            value={
              <span
                data-testid="history-detail-preset"
                {...(isBuiltinPreset(entry.preset.id) ? {} : { "data-user-text": "" })}>
                {presetRefLabel(entry.preset, locale)}
              </span>
            }
            size="sm"
          />
        )}
        <Readout
          label={t("history.detail.outcome")}
          value={outcomeDetailText(entry.outcome, locale)}
          size="sm"
        />
        <Readout
          label={t("history.detail.duration")}
          value={formatTakeLength(entry.duration_ms, locale)}
          size="sm"
        />
        <Readout
          label={t("history.detail.chars")}
          value={t("count.chars", { n: textChars(entry.text) })}
          size="sm"
        />
        <Readout label={t("history.detail.id")} value={entry.id} size="sm" />
        {entry.app !== undefined && (
          <>
            <Readout
              label={t("history.context.app")}
              value={
                <span data-user-text data-testid="history-detail-app">
                  {entry.app.name} · {entry.app.id}
                </span>
              }
              size="sm"
            />
            <Readout
              label={t("history.context.scene")}
              value={
                entry.scene === undefined ? (
                  t("history.context.noScene")
                ) : (
                  <span
                    data-testid="history-detail-scene"
                    {...(entry.scene.builtin === undefined ? { "data-user-text": "" } : {})}>
                    {sceneLabel(entry.scene, locale)}
                  </span>
                )
              }
              muted={entry.scene === undefined}
              size="sm"
            />
          </>
        )}
      </div>
      <div className="mt-auto flex flex-wrap items-center gap-2 border-t border-border pt-3">
        <Button size="sm" onClick={onCopy}>
          {t("common.copy")}
        </Button>
        <Keycaps keys="Ctrl C" />
        <IconButton
          icon="star"
          label={starLabel}
          onClick={onStar}
          className={entry.starred ? "text-fg" : ""}
        />
        {(fixing || entry.kind === "edit") && (
          <Button
            size="sm"
            icon="book"
            onMouseDown={(e) => {
              // Keep the text selection: it pre-fills the misheard form.
              e.preventDefault();
            }}
            onClick={(e) => {
              openFromButton(e.currentTarget);
            }}>
            {t("history.addToDictionary.button")}
          </Button>
        )}
        <Button size="sm" variant="text-danger" className="ml-auto" onClick={onRemove}>
          {t("common.delete")}
        </Button>
        <Keycaps keys="Del" />
      </div>

      {live !== undefined && card === undefined && (
        <SelectionToolbar
          rect={live.rect}
          block={live.target.block}
          onDictionary={() => {
            openFromSelection(live, "dictionary");
          }}
          onRule={() => {
            openFromSelection(live, "rule");
          }}
        />
      )}
      {tip !== undefined && card === undefined && (
        <MarkTip
          el={tip.el}
          text={
            tip.mark.kind === "known"
              ? t("history.fix.marks.knownTooltip", { term: tip.mark.entry?.term ?? "" })
              : t("history.fix.marks.tooltip", { polished: tip.mark.pair?.polished ?? "" })
          }
        />
      )}
      {card !== undefined && (
        <div data-fix-panel="" className="contents">
          {card.open.mode === "known" ? (
            <KnownCard
              key={card.seq}
              mark={card.open.mark}
              anchor={cardAnchor}
              onClose={closeCard}
              onOpenDictionary={openDictionary}
            />
          ) : (
            <QuickFixCard
              key={card.seq}
              entry={entry}
              open={card.open}
              anchor={cardAnchor}
              onClose={closeCard}
              onEdit={() => {
                dirty.current = true;
              }}
              onSaved={onSaved}
              onOpenDictionary={openDictionary}
            />
          )}
        </div>
      )}
    </div>
  );
}

/** The marks' legend under the original text (§16.9 3.1, board 01 and 06). */
function MarkLegend({ polish, known }: { polish: number; known: number }) {
  const { t } = useI18n();
  return (
    <div
      className="mt-3 flex flex-wrap items-center gap-x-5 gap-y-1 text-[12px] text-fg-muted"
      data-testid="fix-legend">
      {polish > 0 && (
        <span className="flex items-center gap-1.5">
          <span aria-hidden className="w-[18px] border-b-2 border-dotted border-primary" />
          {t("history.fix.marks.polishedCount", { n: polish })}
        </span>
      )}
      {known > 0 && (
        <span className="flex items-center gap-1.5">
          <span aria-hidden className="h-[1.5px] w-4 bg-ok-text" />
          {t("history.fix.marks.knownCount", { n: known })}
        </span>
      )}
      <span className="text-fg-subtle">· {t("history.fix.marks.hint")}</span>
    </div>
  );
}

/** A mark's tooltip, above it (§16.9 3.1): what the polish wrote, or the right spelling. */
function MarkTip({ el, text }: { el: HTMLElement; text: string }) {
  const panel = useRef<HTMLDivElement>(null);
  const anchor = useCallback(() => boxOf(el), [el]);
  const { style } = useFloatingStyle(panel, anchor, { side: "top", align: "center" });
  return (
    <div
      ref={panel}
      aria-hidden
      data-testid="fix-tip"
      style={style}
      className="pointer-events-none z-40 rounded-[9px] border border-raised-edge bg-raised px-2.5 py-1 text-[12px] whitespace-nowrap text-fg shadow-pop"
      data-user-text>
      {text}
    </div>
  );
}
