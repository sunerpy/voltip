// One entry of a computer's history on the phone (apps/mobile's MirrorEntry, docs/dictation.md
// §20.8): read-only — the text as polished, as recognised and as processed, when and how it was
// made and where it came from; copy and share, which a text over `MAX_PASTE_TEXT_CHARS` cannot take
// whole. An entry that arrived shortened says so.
import {
  MAX_PASTE_TEXT_CHARS,
  type MirrorEntry as MirrorEntryData,
  dayLabel,
  errorText,
  formatCount,
  formatDuration,
  outcomeLabel,
  presetRefLabel,
  refineModelText,
  sceneLabel,
  shortClockLabel,
  textChars,
} from "@voltip/shared";
import { useEffect, useState } from "react";
import { View } from "react-native";
import { SegmentedButtons } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useNow } from "../backend/useNow";
import { useRootRoute } from "../routes";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { EmptyState, FactRow, Hint, Notice, Page, Rows, Section } from "../ui/kit";
import { EntryHeader, EntryText } from "./Entry";

type TextView = "polished" | "raw" | "processed";

/** The entry `id` of the copy of `desktop` (`mirror_history_entry`), asked again whenever the copy
 *  changes: `undefined` until the answer, `null` once it is gone. */
function useMirrorEntry(desktop: string, id: string): MirrorEntryData | null | undefined {
  const { backend } = useBackend();
  const copy = useUiState().mirrors.find((m) => m.desktop === desktop);
  const revision = `${copy?.state}:${copy?.entries}:${copy?.synced_at_ms}`;
  const [answer, setAnswer] = useState<{ key: string; entry: MirrorEntryData | null } | undefined>(
    undefined,
  );
  const key = `${desktop}/${id}`;
  useEffect(() => {
    let live = true;
    backend.mirrorHistoryEntry(desktop, id).then(
      (entry) => {
        if (live) setAnswer({ key, entry });
      },
      () => {
        if (live) setAnswer({ key, entry: null });
      },
    );
    return () => {
      live = false;
    };
    // The copy changed (a batch arrived, the computer deleted it): ask again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [backend, desktop, id, key, revision]);
  return answer?.key === key ? answer.entry : undefined;
}

export function MirrorEntry() {
  const shell = useShell();
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const now = useNow() * 1000;
  const { desktop, id } = useRootRoute<"MirrorEntry">().params;
  const found = useMirrorEntry(desktop, id);
  const [view, setView] = useState<TextView>("polished");

  if (found === undefined) return <Page>{null}</Page>;
  if (found === null)
    return (
      <Page>
        <Section>
          <EmptyState icon="delete-outline" title={t("history.long.exportFailed.gone")}>
            {t("mobile.entry.goneBody")}
          </EmptyState>
        </Section>
      </Page>
    );

  const { entry, shortened } = found;
  const processed = entry.processed;
  const shown: TextView = view === "processed" && processed === undefined ? "polished" : view;
  const text =
    shown === "raw" ? entry.raw_text : shown === "processed" ? (processed?.text ?? "") : entry.text;
  const tooLong = textChars(text) > MAX_PASTE_TEXT_CHARS;
  const outcome = outcomeLabel(entry.outcome, locale);
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
  const views: { value: TextView; label: string }[] = [
    { value: "polished", label: t("history.view.polished") },
    { value: "raw", label: t("history.view.raw") },
    ...(processed === undefined
      ? []
      : [{ value: "processed" as const, label: t("history.view.processed") }]),
  ];
  return (
    <Page testID="phone-mirror-entry">
      <EntryHeader
        when={`${dayLabel(entry.at_ms, now, locale)} ${shortClockLabel(entry.at_ms)}`}
        outcome={outcome}
      />
      {(entry.refined || processed !== undefined) && (
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
      {shortened && <Notice testID="phone-mirror-entry-shortened">{t("mirror.shortened")}</Notice>}
      <EntryText text={text} testID="phone-mirror-entry-text" />
      <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 8 }}>
        <Button mode="contained-tonal" icon="content-copy" disabled={tooLong} onPress={copy}>
          {t("mobile.recent.copy")}
        </Button>
        <Button
          mode="contained-tonal"
          icon="share-variant-outline"
          disabled={tooLong}
          onPress={() => void backend.invoke("phone_share_text", { text }).catch(fail)}>
          {t("mobile.recent.share")}
        </Button>
      </View>
      {tooLong && <Hint>{t("mirror.tooLong")}</Hint>}
      <Hint>{t("mirror.readOnly")}</Hint>
      <Section>
        <Rows>
          {entry.origin !== undefined && (
            <FactRow
              label={t("history.detail.origin")}
              value={t(`history.origin.${entry.origin.kind}`, { device: entry.origin.device })}
            />
          )}
          <FactRow
            label={t("history.detail.duration")}
            value={formatDuration(entry.duration_ms, locale)}
          />
          <FactRow label={t("history.detail.chars")} value={String(textChars(entry.text))} mono />
          <FactRow label={t("history.detail.asrModel")} value={entry.asr_model} mono />
          <FactRow
            label={t("history.detail.refineModel")}
            value={
              entry.refine_model === undefined ? refineModelText(entry, locale) : entry.refine_model
            }
            mono={entry.refine_model !== undefined}
          />
          {entry.preset !== undefined && (
            <FactRow
              label={t("history.detail.preset")}
              value={presetRefLabel(entry.preset, locale)}
            />
          )}
          {entry.app !== undefined && (
            <FactRow label={t("history.context.app")} value={entry.app.name} />
          )}
          {entry.scene !== undefined && (
            <FactRow label={t("history.context.scene")} value={sceneLabel(entry.scene, locale)} />
          )}
          <FactRow
            label={t("mobile.entry.time")}
            value={t("history.timing.total", {
              n: formatCount(entry.asr_ms + (entry.refine_ms ?? 0)),
            })}
          />
        </Rows>
      </Section>
    </Page>
  );
}
