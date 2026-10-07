// One entry of the phone's history (apps/mobile's HistoryEntry, docs/dictation.md §20.7): its text
// as polished, as recognised and, after 用 AI 预设处理, as processed; when and how it was made; copy,
// share, star and delete (after a confirmation); and for a long entry the processing and the
// exports, which go to the share sheet as files.
import {
  BUILTIN_PRESETS,
  type ExportFormat,
  type HistoryEntry as HistoryEntryData,
  dayLabel,
  errorText,
  formatCount,
  formatDuration,
  outcomeLabel,
  presetLabel,
  presetRefLabel,
  refineModelText,
  sceneLabel,
  shortClockLabel,
  textChars,
} from "@voltip/shared";
import { useEffect, useState } from "react";
import { View } from "react-native";
import { ProgressBar, SegmentedButtons, Text } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useNow } from "../backend/useNow";
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
  type Tone,
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

function outcomeTone(tone: string): Tone {
  return tone === "ok" || tone === "accent" || tone === "danger" || tone === "warning"
    ? tone
    : "idle";
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
      <StateLine tone={outcomeTone(outcome.tone)} small>
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
  const tooLarge = useUiState().phone_outbox_too_large.includes(id);
  const process = useHistoryProcess(id);
  const [view, setView] = useState<TextView>("polished");

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
  const views: { value: TextView; label: string }[] = [
    { value: "polished", label: t("history.view.polished") },
    { value: "raw", label: t("history.view.raw") },
    ...(processed === undefined
      ? []
      : [{ value: "processed" as const, label: t("history.view.processed") }]),
  ];

  return (
    <Page testID="phone-entry">
      <EntryHeader
        when={`${dayLabel(entry.at_ms, now, locale)} ${shortClockLabel(entry.at_ms)}`}
        outcome={outcome}
      />
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
        <View style={{ flex: 1 }} />
        <Button textColor={theme.colors.error} onPress={remove}>
          {t("common.delete")}
        </Button>
      </View>
      {tooLarge && <Hint>{t("mobile.entry.tooLarge")}</Hint>}
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
