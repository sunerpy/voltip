// One entry of the phone's history (apps/mobile's HistoryEntry, docs/dictation.md §20.7): its text
// as polished, as recognised and, after 用 AI 预设处理, as processed; when and how it was made; copy,
// share, star and delete (after a confirmation); for a long entry the processing and the exports,
// which go to the share sheet as files; and for a take this phone recognised, 纠正识别错误 (§16.9 4):
// its original text as words to pick, and what to make of them (features/QuickFix.tsx).
import { usePreventRemove } from "@react-navigation/native";
import {
  BUILTIN_PRESETS,
  type ExportFormat,
  type FixKind,
  type HistoryEntry as HistoryEntryData,
  type TextRange,
  type WordPiece,
  dayLabel,
  errorText,
  fixScope,
  fixTarget,
  formatCount,
  formatDuration,
  outcomeLabel,
  prefillFor,
  presetLabel,
  presetRefLabel,
  refineModelText,
  sceneLabel,
  serviceReasonText,
  shortClockLabel,
  takePolishPairs,
  tapPiece,
  textChars,
  textMarks,
  wordPieces,
  wordSegmenter,
} from "@voltip/shared";
import { useCallback, useEffect, useLayoutEffect, useMemo, useState } from "react";
import { View } from "react-native";
import { ProgressBar, SegmentedButtons, Text } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useNow } from "../backend/useNow";
import {
  FIX_BAR_SPACE,
  FixBar,
  FixLegend,
  type FixOpen,
  FixSheet,
  FixWords,
  type SavedFix,
} from "../features/QuickFix";
import { exportName, isLongEntry, useHistoryProcess } from "../hooks/useHistoryProcess";
import { useRootNavigation, useRootRoute } from "../routes";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import {
  EmptyState,
  FactRow,
  Hint,
  Mono,
  Page,
  Rows,
  Section,
  SectionTitle,
  StateLine,
  labelTone,
  useAppTheme,
} from "../ui/kit";
import { SelectField } from "../ui/Select";

type TextView = "polished" | "raw" | "processed";

/** The entry `id` from the core (`history_entry`), asked again on every history event: `undefined`
 *  until the answer, `null` once it is gone. */
function useEntry(id: string): HistoryEntryData | null | undefined {
  const { backend } = useBackend();
  const { history_recent: revision } = useUiState();
  const [answer, setAnswer] = useState<{ id: string; entry: HistoryEntryData | null } | undefined>(
    undefined,
  );
  useEffect(() => {
    let live = true;
    backend.historyEntry(id).then(
      (entry) => {
        if (live) setAnswer({ id, entry });
      },
      () => {
        if (live) setAnswer({ id, entry: null });
      },
    );
    return () => {
      live = false;
    };
    // A history event (a star, a deletion, a processed text) is a reason to ask again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [backend, id, revision]);
  return answer?.id === id ? answer.entry : undefined;
}

/** The day, the time and how the take ended, above its text. */
export function EntryHeader({
  when,
  outcome,
}: {
  when: string;
  outcome: ReturnType<typeof outcomeLabel>;
}) {
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 4 }}>
      <SectionTitle style={{ flex: 1 }}>{when}</SectionTitle>
      <StateLine tone={labelTone(outcome.tone)} small testID="entry-outcome">
        {outcome.text}
      </StateLine>
    </View>
  );
}

/** The text of an entry as the chosen view shows it: selectable, in its own container. */
export function EntryText({ text, testID }: { text: string; testID: string }) {
  return (
    <Section padded>
      <Text variant="bodyLarge" selectable testID={testID} style={{ lineHeight: 28 }}>
        {text}
      </Text>
    </Section>
  );
}

function LongTools({
  entry,
  process,
}: {
  entry: HistoryEntryData;
  process: ReturnType<typeof useHistoryProcess>;
}) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const { settings, presets } = useUiState();
  const [preset, setPreset] = useState<string>(settings.engines.refine_preset);
  const view = process.view;
  const running = view.state === "running";
  const processed = view.state === "done" ? view.processed : entry.processed;
  const hasSegments = (entry.segments ?? []).some((s) => s.text.trim().length > 0);
  const share = async (format: ExportFormat) => {
    const outcome = await backend.historyExport(entry.id, format, exportName(entry.at_ms));
    if (outcome.kind === "failed")
      shell.toast(
        t(`history.long.exportFailed.${outcome.code}`, { detail: outcome.detail }),
        "danger",
      );
  };
  return (
    <Section title={t("history.long.title")} padded testID="phone-entry-long">
      <SelectField
        label={t("history.long.preset")}
        value={preset}
        disabled={running}
        options={[
          ...BUILTIN_PRESETS.map((id) => ({ value: id, label: presetLabel(id, presets, locale) })),
          ...presets.map((p) => ({ value: p.id, label: p.name })),
        ]}
        onChange={setPreset}
      />
      {running ? (
        <Button mode="outlined" onPress={process.cancel}>
          {t("common.cancel")}
        </Button>
      ) : (
        <Button
          mode="contained"
          onPress={() => {
            process.start(preset);
          }}>
          {processed === undefined ? t("history.long.start") : t("history.long.again")}
        </Button>
      )}
      {running && (
        <View style={{ gap: 4 }}>
          <ProgressBar
            progress={view.total === 0 ? 0 : view.done / view.total}
            indeterminate={view.total === 0}
          />
          <Mono>{t("history.long.running", { done: view.done, total: view.total })}</Mono>
        </View>
      )}
      {view.state === "failed" && (
        <Hint tone="danger">{t("history.long.failed", { reason: view.reason })}</Hint>
      )}
      {view.state === "cancelled" && <Hint>{t("history.long.cancelled")}</Hint>}
      <Text variant="bodySmall" style={{ color: theme.voltip.subtle }}>
        {t("history.long.note")}
      </Text>
      <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 8 }}>
        <Button
          mode="outlined"
          icon="share-variant-outline"
          disabled={!hasSegments}
          onPress={() => void share("srt")}>
          {t("mobile.entry.shareSrt")}
        </Button>
        <Button mode="outlined" icon="share-variant-outline" onPress={() => void share("txt")}>
          {t("mobile.entry.shareTxt")}
        </Button>
      </View>
      {processed !== undefined && <Hint>{t("history.long.txtUsesProcessed")}</Hint>}
    </Section>
  );
}

export function Entry() {
  const theme = useAppTheme();
  const shell = useShell();
  const navigation = useRootNavigation();
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const now = useNow() * 1000;
  const id = useRootRoute<"Entry">().params.id;
  const entry = useEntry(id);
  const ui = useUiState();
  const tooLarge = ui.phone_outbox_too_large.includes(id);
  const process = useHistoryProcess(id);
  const [view, setView] = useState<TextView>("polished");

  // 纠正识别错误 (§16.9 4): only for a take this phone recognised, with words in its original text.
  const scope = entry == null ? undefined : fixScope(entry, "phone");
  const canFix = entry != null && scope?.kind === "fix" && entry.raw_text.trim() !== "";
  const [fixing, setFixing] = useState(false);
  const active = fixing && canFix;
  const [selection, setSelection] = useState<TextRange | undefined>(undefined);
  const [sheet, setSheet] = useState<FixOpen | undefined>(undefined);
  const [sheetOpen, setSheetOpen] = useState(false);
  const raw = entry?.raw_text ?? "";
  // The replacements the AI polish made: none when the take was not polished, and none that its
  // own corrections and rules account for (§16.9 3.1).
  const pairs = useMemo(
    () => (active && entry != null ? takePolishPairs(entry, ui.dictionary, ui.rules) : []),
    [active, entry, ui.dictionary, ui.rules],
  );
  const marks = useMemo(
    () => (active ? textMarks(raw, pairs, ui.dictionary) : []),
    [active, raw, pairs, ui.dictionary],
  );
  // Hermes has no `Intl.Segmenter`: on the phone the words are split by script (quick-fix.ts).
  const pieces = useMemo(
    () => (active ? wordPieces(raw, marks, wordSegmenter()) : []),
    [active, raw, marks],
  );
  const exitFix = useCallback(() => {
    setFixing(false);
    setSelection(undefined);
    setSheetOpen(false);
  }, []);
  const tap = useCallback((piece: WordPiece) => {
    setSelection((was) => tapPiece(was, piece));
  }, []);
  // The selection made fit for the sheet (grown to a whole replacement, its default kind).
  const target = useMemo(
    () =>
      entry != null && active && selection !== undefined
        ? fixTarget(entry, "raw", selection.start, selection.end, pairs)
        : undefined,
    [entry, active, selection, pairs],
  );
  // The bar's title and 完成 while words are being picked.
  useLayoutEffect(() => {
    navigation.setOptions(
      active
        ? {
            title: t("mobile.entry.fix"),
            headerRight: () => (
              <Button testID="entry-fix-done" onPress={exitFix}>
                {t("common.done")}
              </Button>
            ),
          }
        : { title: t("mobile.title.entry"), headerRight: undefined },
    );
  }, [navigation, active, exitFix, t]);
  // Back leaves the picking first (an open sheet closes before that, on its own).
  usePreventRemove(active, exitFix);

  if (entry === undefined) return <Page>{null}</Page>;
  if (entry === null)
    return (
      <Page>
        <Section>
          <EmptyState icon="delete-outline" title={t("history.long.exportFailed.gone")}>
            {t("mobile.entry.goneBody")}
          </EmptyState>
        </Section>
      </Page>
    );

  const processed = process.view.state === "done" ? process.view.processed : entry.processed;
  const shown: TextView = view === "processed" && processed === undefined ? "polished" : view;
  const text =
    shown === "raw" ? entry.raw_text : shown === "processed" ? (processed?.text ?? "") : entry.text;
  const outcome = outcomeLabel(entry.outcome, locale);
  // A take a computer delivered: the phone has the text the computer reported and the length of the
  // audio; the models and timings are in the computer's history.
  const sent = entry.origin?.kind === "sent";
  const fail = (e: unknown) => {
    shell.toast(t("mobile.toast.error", { message: errorText(e) }), "danger");
  };
  const copy = () => {
    void backend.pasteText(text).then(
      (answer) => {
        if (answer.kind === "failed") shell.toast(t("mobile.recent.copyFailed"), "danger");
        else shell.toast(t("history.detail.copied", { n: textChars(text) }));
      },
      () => {
        shell.toast(t("mobile.recent.copyFailed"), "danger");
      },
    );
  };
  const remove = () => {
    shell.confirm({
      title: t("history.confirm.deleteTitle"),
      body: t("history.confirm.deleteBody", {
        when: `${dayLabel(entry.at_ms, now, locale)} ${shortClockLabel(entry.at_ms)}`,
        excerpt: `${entry.text.slice(0, 40)}${entry.text.length > 40 ? "…" : ""}`,
      }),
      confirmLabel: t("common.delete"),
      onConfirm: () => {
        backend.invoke("history_delete", { id: entry.id }).then(() => {
          navigation.goBack();
        }, fail);
      },
    });
  };
  const openSheet = (kind: FixKind) => {
    if (target === undefined) return;
    const prefill = { ...prefillFor(target), kind };
    setSheet((was) => ({ id: (was?.id ?? 0) + 1, kind, prefill }));
    setSheetOpen(true);
  };
  const closeSheet = () => {
    setSheetOpen(false);
  };
  const saved = (fix: SavedFix) => {
    setSheetOpen(false);
    setSelection(undefined);
    shell.toast(fix.message, "neutral", {
      label: t("history.fix.undo"),
      onPress: () => {
        // The lists as they are now: the entry or rule the save made is found by what it is.
        backend.getState().then(fix.undo).catch(fail);
      },
    });
  };
  const openDictionary = () => {
    setSheetOpen(false);
    navigation.navigate("Dictionary");
  };
  const views: { value: TextView; label: string }[] = [
    { value: "polished", label: t("history.view.polished") },
    { value: "raw", label: t("history.view.raw") },
    ...(processed === undefined
      ? []
      : [{ value: "processed" as const, label: t("history.view.processed") }]),
  ];

  const header = (
    <EntryHeader
      when={`${dayLabel(entry.at_ms, now, locale)} ${shortClockLabel(entry.at_ms)}`}
      outcome={outcome}
    />
  );
  if (active) {
    return (
      <View style={{ flex: 1 }}>
        <Page testID="phone-entry-fix">
          {header}
          <SectionTitle style={{ paddingHorizontal: 4 }}>{t("history.view.raw")}</SectionTitle>
          <FixWords pieces={pieces} selection={selection} onTap={tap} />
          <FixLegend marks={marks} />
          {target !== undefined && <View style={{ height: FIX_BAR_SPACE }} />}
        </Page>
        {target !== undefined && (
          <FixBar
            target={target}
            onClear={() => {
              setSelection(undefined);
            }}
            onOpen={openSheet}
          />
        )}
        <FixSheet
          open={sheetOpen}
          spec={sheet}
          entry={entry}
          onClose={closeSheet}
          onSaved={saved}
          onOpenDictionary={openDictionary}
        />
      </View>
    );
  }

  return (
    <Page testID="phone-entry">
      {header}
      {entry.refined && (
        <SegmentedButtons
          value={shown}
          onValueChange={(v) => {
            setView(v);
          }}
          buttons={views}
        />
      )}
      {shown === "processed" && processed !== undefined && (
        <Hint>{t("history.view.processedBy", { preset: processed.preset.name })}</Hint>
      )}
      <EntryText text={text} testID="phone-entry-text" />
      <View style={{ flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: 8 }}>
        <Button mode="contained-tonal" icon="content-copy" onPress={copy}>
          {t("mobile.recent.copy")}
        </Button>
        <Button
          mode="contained-tonal"
          icon="share-variant-outline"
          onPress={() => void backend.invoke("phone_share_text", { text }).catch(fail)}>
          {t("mobile.recent.share")}
        </Button>
        <Button
          mode={entry.starred ? "contained" : "outlined"}
          icon={entry.starred ? "star" : "star-outline"}
          accessibilityState={{ selected: entry.starred }}
          onPress={() =>
            void backend
              .invoke("history_star", { id: entry.id, starred: !entry.starred })
              .catch(fail)
          }>
          {entry.starred ? t("history.unstar") : t("history.star")}
        </Button>
        {canFix && (
          <Button
            mode="contained-tonal"
            icon="spellcheck"
            testID="entry-fix"
            onPress={() => {
              setFixing(true);
            }}>
            {t("mobile.entry.fix")}
          </Button>
        )}
        <View style={{ flex: 1 }} />
        <Button textColor={theme.colors.error} onPress={remove}>
          {t("common.delete")}
        </Button>
      </View>
      {tooLarge && <Hint>{t("mobile.entry.tooLarge")}</Hint>}
      {scope?.kind === "note" && (
        <Hint testID="entry-fix-note">{t(`history.fix.notHere.${scope.note}`)}</Hint>
      )}
      {isLongEntry(entry) && <LongTools entry={entry} process={process} />}
      <Section>
        <Rows>
          {sent && (
            <FactRow
              label={t("history.detail.origin")}
              value={t("history.origin.sent", { device: entry.origin?.device ?? "" })}
            />
          )}
          <FactRow
            label={t("history.detail.duration")}
            value={formatDuration(entry.duration_ms, locale)}
          />
          <FactRow label={t("history.detail.chars")} value={String(textChars(entry.text))} mono />
          {!sent && <FactRow label={t("history.detail.asrModel")} value={entry.asr_model} mono />}
          {!sent && (
            <FactRow
              label={t("history.detail.refineModel")}
              value={
                entry.refine_model === undefined
                  ? refineModelText(entry, locale)
                  : entry.refine_model
              }
              mono={entry.refine_model !== undefined}
            />
          )}
          {!sent && !entry.refined && entry.refine_error !== undefined && (
            // docs/dictation.md §3.12: why the polish failed, with what the service said.
            <FactRow
              label={t("history.refineNote.reasonLabel")}
              value={serviceReasonText(entry.refine_error, locale)}
              mono
            />
          )}
          {!sent && entry.preset !== undefined && (
            <FactRow
              label={t("history.detail.preset")}
              value={presetRefLabel(entry.preset, locale)}
            />
          )}
          {!sent && entry.scene !== undefined && (
            <FactRow label={t("history.context.scene")} value={sceneLabel(entry.scene, locale)} />
          )}
          {!sent && (
            <FactRow
              label={t("mobile.entry.time")}
              value={t("history.timing.total", {
                n: formatCount(entry.asr_ms + (entry.refine_ms ?? 0)),
              })}
            />
          )}
        </Rows>
      </Section>
      {sent && <Hint>{t("mobile.entry.sentNote")}</Hint>}
    </Page>
  );
}
