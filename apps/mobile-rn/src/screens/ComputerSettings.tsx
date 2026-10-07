// 设置 › 电脑 › a computer's settings (apps/mobile's ComputerSettings, docs/dictation.md §20.8):
// read-only — its language and theme as set there, its recognition and AI polish, its presets,
// dictionary, rules and scenes, as the copy last received them.
import { type MirrorProfile, mirrorStateText, presetLabel, sceneLabel } from "@voltip/shared";
import { useEffect, useState } from "react";
import { View } from "react-native";
import { Text } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useNow } from "../backend/useNow";
import { useRootRoute } from "../routes";
import {
  EmptyState,
  FactRow,
  Lede,
  Mono,
  Page,
  RowDivider,
  Rows,
  Section,
  useAppTheme,
} from "../ui/kit";

/** The settings the copy of `desktop` holds (`mirror_profile`), asked again whenever the copy
 *  changes: `undefined` until the answer, `null` while none arrived. */
function useProfile(desktop: string): MirrorProfile | null | undefined {
  const { backend } = useBackend();
  const copy = useUiState().mirrors.find((m) => m.desktop === desktop);
  const revision = `${copy?.state}:${copy?.synced_at_ms}`;
  const [answer, setAnswer] = useState<{ desktop: string; profile: MirrorProfile | null }>();
  useEffect(() => {
    let live = true;
    backend.mirrorProfile(desktop).then(
      (profile) => {
        if (live) setAnswer({ desktop, profile });
      },
      () => {
        if (live) setAnswer({ desktop, profile: null });
      },
    );
    return () => {
      live = false;
    };
    // The copy changed (new settings arrived, or it was deleted): ask again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [backend, desktop, revision]);
  return answer?.desktop === desktop ? answer.profile : undefined;
}

function Items({
  items,
  none,
}: {
  items: { key: string; title: string; detail?: string }[];
  none: string;
}) {
  const theme = useAppTheme();
  if (items.length === 0)
    return (
      <Text variant="bodyMedium" style={{ padding: 16, color: theme.colors.onSurfaceVariant }}>
        {none}
      </Text>
    );
  return items.map((item, i) => (
    <View key={item.key}>
      {i > 0 && <RowDivider />}
      <View style={{ paddingHorizontal: 16, paddingVertical: 12, gap: 2 }}>
        <Text variant="titleSmall">{item.title}</Text>
        {item.detail !== undefined && item.detail.length > 0 && (
          <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
            {item.detail}
          </Text>
        )}
      </View>
    </View>
  ));
}

export function ComputerSettings() {
  const { t, locale } = useI18n();
  const now = useNow();
  const { desktop } = useRootRoute<"ComputerSettings">().params;
  const copy = useUiState().mirrors.find((m) => m.desktop === desktop);
  const profile = useProfile(desktop);
  const name = copy?.name ?? "";
  if (profile === undefined) return <Page>{null}</Page>;
  return (
    <Page testID="phone-computer-settings" gap={20}>
      <View style={{ gap: 4 }}>
        <Lede>{t("mirror.settings.lede", { name })}</Lede>
        {copy !== undefined && (
          <Mono style={{ paddingHorizontal: 4 }}>{mirrorStateText(copy, now, locale)}</Mono>
        )}
      </View>
      {profile === null ? (
        <Section>
          <EmptyState icon="cloud-off-outline" title={t("mirror.settings.none")} />
        </Section>
      ) : (
        <>
          <Section title={t("mirror.settings.look")}>
            <Rows>
              <FactRow
                label={t("mirror.settings.locale")}
                value={t(`settings.general.locale.${profile.locale}`)}
              />
              <FactRow
                label={t("mirror.settings.theme")}
                value={
                  profile.follow_system_theme
                    ? t("theme.followSystem")
                    : t(`theme.name.${profile.theme}`)
                }
              />
            </Rows>
          </Section>
          <Section title={t("mirror.settings.speech")}>
            <FactRow
              label={t("mirror.settings.speechModel")}
              value={t("mobile.settings.serviceDetail", {
                provider: t(`engines.provider.${profile.asr_provider}`),
                model: profile.asr_model.length > 0 ? profile.asr_model : "—",
              })}
            />
          </Section>
          <Section title={t("mirror.settings.polish")}>
            <Rows>
              <FactRow
                label={t("mirror.settings.polishState")}
                value={
                  profile.refine_enabled
                    ? t("mirror.settings.polishOn")
                    : t("mirror.settings.polishOff")
                }
              />
              <FactRow
                label={t("mirror.settings.polishModel")}
                value={t("mobile.settings.serviceDetail", {
                  provider:
                    profile.llm_provider === undefined
                      ? "—"
                      : t(`engines.provider.${profile.llm_provider}`),
                  model: profile.refine_model.length > 0 ? profile.refine_model : "—",
                })}
              />
              <FactRow
                label={t("mirror.settings.preset")}
                value={presetLabel(profile.preset, profile.presets, locale)}
              />
            </Rows>
          </Section>
          <Section title={t("mirror.settings.presets")}>
            <Items
              none={t("mirror.settings.none_items")}
              items={profile.presets.map((p) => ({ key: p.id, title: p.name, detail: p.prompt }))}
            />
          </Section>
          <Section title={t("mirror.settings.dictionary")}>
            <Items
              none={t("mirror.settings.none_items")}
              items={profile.dictionary.map((d) => ({
                key: d.id,
                title: d.term,
                ...(d.heard_as.length > 0
                  ? { detail: t("mirror.settings.heardAs", { terms: d.heard_as.join("、") }) }
                  : {}),
              }))}
            />
          </Section>
          <Section title={t("mirror.settings.rules")}>
            <Items
              none={t("mirror.settings.none_items")}
              items={profile.rules.map((r) => ({
                key: r.id,
                title: r.name,
                detail: `${r.pattern} → ${r.replacement}`,
              }))}
            />
          </Section>
          <Section title={t("mirror.settings.scenes")}>
            <Items
              none={t("mirror.settings.none_items")}
              items={profile.scenes.map((s) => ({ key: s.id, title: sceneLabel(s, locale) }))}
            />
          </Section>
        </>
      )}
    </Page>
  );
}
