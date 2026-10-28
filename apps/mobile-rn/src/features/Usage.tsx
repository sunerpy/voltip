// The built-in service's usage on the phone (docs/dictation.md §27; Penpot 「用量与额度」 boards
// 15–16): the talk card's last line while the phone recognises with the built-in service, the
// notices under the card from 80 % and at the limit, what the card says while recognition is used
// up, and the parts of 设置 › 用量. The numbers and their texts are `@voltip/shared`'s (usage.ts),
// as on the desktop; the phone counts its own takes (a take sent to a computer is the computer's).
import {
  type ServiceKind,
  type ServiceUsage,
  type UsageNotice,
  type UsageTone,
  providerUseAmount,
  resetCountdown,
  resetLabel,
  serviceUsage,
  usableFallback,
  usageAmount,
  usageDayLabel,
  usageInline,
  usageLeft,
  usageNotice,
  usageOf,
  usageValue,
  usageWeek,
  usesBuiltin,
} from "@voltip/shared";
import { useState } from "react";
import { type LayoutChangeEvent, StyleSheet, View } from "react-native";
import { Icon, Text } from "react-native-paper";
import Svg, { Line } from "react-native-svg";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useNow } from "../backend/useNow";
import { useRootNavigation } from "../routes";
import { Button } from "../ui/Button";
import { Notice, rgba, useAppTheme } from "../ui/kit";

/** The current time in milliseconds, refreshed every 30 s (`useNow`). */
export function useUsageNow(): number {
  return useNow() * 1000;
}

/** `kind`'s usage today, from `UiState.usage`. */
export function useServiceUsage(kind: ServiceKind): ServiceUsage {
  const { usage } = useUiState();
  return serviceUsage(usage, kind, useUsageNow());
}

/** The phone's own takes use the built-in `kind` now, in a build that has it: only then do its
 *  limits apply to them. */
export function useBuiltinInUse(kind: ServiceKind): boolean {
  const { usage, engines } = useUiState();
  return usage.builtin && usesBuiltin(engines, kind);
}

/** The icon of each service, as 设置 shows them. */
export const USAGE_ICON: Readonly<Record<ServiceKind, string>> = {
  asr: "waveform",
  llm: "creation-outline",
};

/** The colour of a state: the accent, amber from 80 %, red at the limit, grey while unknown. */
function useToneColor(): (tone: UsageTone) => string {
  const theme = useAppTheme();
  return (tone) => {
    switch (tone) {
      case "near":
        return theme.voltip.warning;
      case "out":
        return theme.voltip.danger;
      case "unknown":
      case "pending":
        return theme.voltip.subtle;
      case "ok":
      case "unlimited":
        return theme.colors.primary;
    }
  };
}

/** A day's use against its limit: a rounded track filled to `share` (a little use still shows as
 *  a dot), empty while the numbers are unknown. */
export function UsageBar({
  share,
  tone,
  height = 6,
  width,
  label,
  testID,
}: {
  share: number;
  tone: UsageTone;
  height?: number;
  /** A fixed width in dp; the bar fills its row otherwise. */
  width?: number;
  label: string;
  testID?: string;
}) {
  const theme = useAppTheme();
  const color = useToneColor();
  const pct = Math.round(Math.min(1, Math.max(0, share)) * 100);
  const filled = share > 0 && tone !== "unknown" && tone !== "pending";
  return (
    <View
      testID={testID}
      accessibilityRole="progressbar"
      accessibilityLabel={label}
      accessibilityValue={{ min: 0, max: 100, now: pct }}
      style={[
        styles.track,
        {
          height,
          borderRadius: height / 2,
          backgroundColor: rgba(theme.colors.onSurface, 0.08),
        },
        width === undefined ? { alignSelf: "stretch" } : { width },
      ]}>
      {filled && (
        <View
          testID={testID === undefined ? undefined : `${testID}-fill`}
          style={{
            width: `${pct}%`,
            minWidth: height,
            height,
            borderRadius: height / 2,
            backgroundColor: color(tone),
          }}
        />
      )}
    </View>
  );
}

/** The talk card's last line while the phone recognises with the built-in service (board 15a):
 *  today's numbers and a short bar; amber from 80 %. Nothing before the first report. */
export function TalkUsageLine() {
  const theme = useAppTheme();
  const { t, locale } = useI18n();
  const usage = useServiceUsage("asr");
  const now = useUsageNow();
  const color = useToneColor();
  if (!useBuiltinInUse("asr") || usage.tone === "pending") return null;
  const text = t("usage.builtinLine", { usage: usageInline(usage, now, locale) });
  const strong = usage.tone === "near" || usage.tone === "out";
  const ink = strong ? color(usage.tone) : theme.voltip.subtle;
  return (
    <View
      testID="talk-usage"
      accessibilityLabel={text}
      style={[styles.talkLine, { borderTopColor: theme.voltip.cardEdge }]}>
      <Icon source="waveform" size={14} color={ink} />
      <Text
        testID="talk-usage-text"
        numberOfLines={1}
        style={[styles.talkText, { color: ink, fontWeight: strong ? "500" : "400" }]}>
        {text}
      </Text>
      {(usage.tone === "ok" || usage.tone === "near") && (
        <UsageBar
          testID="talk-usage-bar"
          share={usage.share}
          tone={usage.tone}
          width={64}
          label={text}
        />
      )}
    </View>
  );
}

/** The built-in recognition is used up and nothing stands in for it (board 15c): the phone keeps
 *  no recordings, so a take would be lost; the card offers the ways that still recognise. `undefined`
 *  while the phone can still recognise. */
export function useRecognitionUsedUp(): ServiceUsage | undefined {
  const { engines } = useUiState();
  const usage = useServiceUsage("asr");
  const now = useUsageNow();
  const inUse = useBuiltinInUse("asr");
  if (!inUse || usage.tone !== "out") return undefined;
  if (usableFallback(engines.asr_fallback, now) !== undefined) return undefined;
  return usage;
}

/** What the talk card says instead of its label while recognition is used up (board 15c): when the
 *  day starts again, and the ways on: a computer online recognises it, another model does. */
export function RecognitionUsedUp({
  usage,
  desktop,
}: {
  usage: ServiceUsage;
  /** The computer the take could go to instead, when one is online. */
  desktop?: { name: string; choose: () => void } | undefined;
}) {
  const theme = useAppTheme();
  const { t, locale } = useI18n();
  const navigation = useRootNavigation();
  const now = useUsageNow();
  const reset = resetLabel(usage.resetAtMs ?? now, now, locale);
  return (
    <View testID="talk-usage-out" style={styles.out}>
      <Text style={[styles.outTitle, { color: theme.colors.onSurface }]}>
        {t("mobile.usage.out.title")}
      </Text>
      <Text style={[styles.outBody, { color: theme.voltip.subtle }]}>
        {desktop === undefined
          ? t("mobile.usage.out.alone", { reset })
          : t("mobile.usage.out.body", { reset })}
      </Text>
      <View style={styles.outActions}>
        {desktop !== undefined && (
          <Button mode="contained" compact testID="talk-usage-to-desktop" onPress={desktop.choose}>
            {t("mobile.usage.out.toDesktop", { name: desktop.name })}
          </Button>
        )}
        <Button
          mode="outlined"
          compact
          testID="talk-usage-switch"
          onPress={() => {
            navigation.navigate("Speech");
          }}>
          {t("mobile.usage.out.switch")}
        </Button>
      </View>
    </View>
  );
}

/** The notices under the talk card while the phone recognises (boards 15b/15c): the built-in
 *  recognition or clean-up near its day's limit (from 80 %) or at it, while it is the one in use.
 *  Each says when the day starts again and what can be done meanwhile; closed, it stays away for
 *  the rest of the day (`usage_notice_dismiss`). */
export function UsageNotices({ online }: { online: boolean }) {
  return (
    <>
      <ServiceNotice kind="asr" online={online} />
      <ServiceNotice kind="llm" online={online} />
    </>
  );
}

function ServiceNotice({ kind, online }: { kind: ServiceKind; online: boolean }) {
  const theme = useAppTheme();
  const { t, locale } = useI18n();
  const { backend } = useBackend();
  const navigation = useRootNavigation();
  const state = useUiState();
  const now = useUsageNow();
  const usage = serviceUsage(state.usage, kind, now);
  // The clean-up counts only while it runs on the phone's takes.
  const inUse =
    usesBuiltin(state.engines, kind) && (kind === "asr" || state.engines.refine_enabled);
  const notice: UsageNotice | undefined = usageNotice(usage, state.usage, inUse);
  if (notice === undefined) return null;
  const resetAt = usage.resetAtMs ?? now;
  const reset = resetLabel(resetAt, now, locale);
  const countdown = resetCountdown(resetAt, now, locale);
  const amount = usage.leftUnderOne
    ? t("usage.underOneMinute")
    : usageAmount(kind, usage.left ?? 0, locale);
  const fallback = usableFallback(
    kind === "asr" ? state.engines.asr_fallback : state.engines.llm_fallback,
    now,
  );
  const out = usage.tone === "out";
  let body: string;
  if (notice === "asr_near") {
    body = online
      ? t("mobile.usage.notice.asr_near.body", { reset })
      : t("mobile.usage.notice.asr_near.alone", { reset });
  } else if (notice === "asr_out") {
    body =
      fallback !== undefined
        ? t("mobile.usage.notice.asr_out.fallback", { reset, model: fallback.model })
        : online
          ? t("mobile.usage.notice.asr_out.body", { countdown })
          : t("mobile.usage.notice.asr_out.alone", { countdown });
  } else if (notice === "refine_out" && fallback !== undefined) {
    body = t("mobile.usage.notice.refine_out.fallback", { reset, model: fallback.model });
  } else {
    body = t(`mobile.usage.notice.${notice}.body`, { reset });
  }
  return (
    <Notice
      tone={out ? "danger" : "warning"}
      icon={out ? "alert-circle-outline" : "gauge"}
      testID={`usage-notice-${notice}`}
      action={
        <View style={styles.noticeActions}>
          <Button
            mode="outlined"
            compact
            testID={`usage-notice-${notice}-open`}
            onPress={() => {
              navigation.navigate(kind === "asr" ? "Speech" : "Ai");
            }}>
            {kind === "asr" ? t("usage.notice.openSpeech") : t("usage.notice.openAi")}
          </Button>
          <Button
            compact
            testID={`usage-notice-${notice}-close`}
            onPress={() => {
              void backend.invoke("usage_notice_dismiss", { notice });
            }}>
            {t("ui.banner.close")}
          </Button>
        </View>
      }>
      <Text variant="titleSmall" style={out ? { color: theme.voltip.danger } : undefined}>
        {t(`mobile.usage.notice.${notice}.title`, { amount })}
      </Text>
      <Text variant="bodyMedium" testID={`usage-notice-${notice}-body`}>
        {body}
      </Text>
    </Notice>
  );
}

/** One service's day on 设置 › 用量 (board 16b): its name and what is left, today's number against
 *  the limit, the bar, when the day starts again, and the last seven days. */
export function ServiceUsageCard({ kind }: { kind: ServiceKind }) {
  const theme = useAppTheme();
  const { t, locale } = useI18n();
  const { backend } = useBackend();
  const { engines } = useUiState();
  const usage = useServiceUsage(kind);
  const now = useUsageNow();
  const color = useToneColor();
  const inUse = usesBuiltin(engines, kind);
  const title = t(`usage.service.${kind}`);
  const status =
    usage.tone === "out"
      ? t("usage.usedUpToday")
      : usage.tone === "unlimited"
        ? t("usage.unlimited")
        : usage.tone === "ok" || usage.tone === "near"
          ? usageLeft(usage, locale)
          : usageValue(usage, locale);
  const reset =
    usage.resetAtMs === undefined
      ? undefined
      : `${resetLabel(usage.resetAtMs, now, locale)} · ${resetCountdown(usage.resetAtMs, now, locale)}`;
  const provider = kind === "asr" ? engines.asr_provider : engines.llm_provider;
  const known = usage.tone !== "unknown" && usage.tone !== "pending";
  return (
    <View
      testID={`usage-card-${kind}`}
      style={[
        styles.card,
        {
          backgroundColor: theme.voltip.card,
          borderColor: theme.voltip.cardEdge,
          borderRadius: theme.voltip.radius.card,
        },
      ]}>
      <View style={styles.cardHead}>
        <Icon
          source={USAGE_ICON[kind]}
          size={18}
          color={usage.tone === "out" ? theme.voltip.danger : theme.colors.primary}
        />
        <Text style={[styles.cardTitle, { color: theme.colors.onSurface }]} numberOfLines={1}>
          {title}
        </Text>
        <Text
          testID={`usage-status-${kind}`}
          style={[
            styles.cardStatus,
            {
              color: usage.tone === "ok" ? theme.colors.onSurface : color(usage.tone),
            },
          ]}>
          {status}
        </Text>
      </View>
      {known ? (
        <>
          <View style={styles.numbers} testID={`usage-numbers-${kind}`}>
            <Text style={[styles.big, { color: theme.colors.onSurface }]}>{usage.used}</Text>
            <Text style={[styles.of, { color: theme.voltip.subtle }]}>
              {usage.limit === undefined
                ? ` ${t(`usage.unit.${kind}`)} · ${t("usage.unlimitedShort")}`
                : ` / ${usageAmount(kind, usage.limit, locale)}`}
            </Text>
          </View>
          {usage.limit !== undefined && (
            <UsageBar
              share={usage.tone === "out" ? 1 : usage.share}
              tone={usage.tone}
              height={8}
              label={`${title} · ${usageOf(usage, locale)}`}
            />
          )}
          {reset !== undefined && (
            <Text
              style={[styles.small, { color: theme.voltip.subtle }]}
              testID={`usage-reset-${kind}`}>
              {reset}
            </Text>
          )}
        </>
      ) : (
        <View style={styles.unknown}>
          <Text style={[styles.small, { color: theme.voltip.subtle, flex: 1 }]}>
            {usage.tone === "unknown" ? t("usage.unavailableBody") : t("usage.pending")}
          </Text>
          {usage.tone === "unknown" && (
            <Button
              compact
              icon="refresh"
              testID={`usage-retry-${kind}`}
              onPress={() => {
                void backend.invoke("usage_refresh");
              }}>
              {t("common.retry")}
            </Button>
          )}
        </View>
      )}
      <View style={styles.week}>
        <Text style={[styles.weekTitle, { color: theme.colors.onSurfaceVariant }]}>
          {t("usage.view.week")}
        </Text>
        <WeekChart kind={kind} {...(usage.limit === undefined ? {} : { limit: usage.limit })} />
      </View>
      {!inUse && (
        <Text
          style={[styles.small, { color: theme.voltip.subtle }]}
          testID={`usage-not-in-use-${kind}`}>
          {t("usage.view.notInUse", {
            service: t(`usage.short.${kind}`),
            provider: provider === undefined ? "—" : t(`engines.provider.${provider}`),
          })}
        </Text>
      )}
    </View>
  );
}

/** The chart's height in dp, its bars at most this wide. */
const CHART_HEIGHT = 72;
const BAR_WIDTH = 28;

/** The last seven days of one service (board 16b): bars against a dashed line at the day's limit,
 *  amber from 80 %, red at the limit, today in the accent. */
export function WeekChart({ kind, limit }: { kind: ServiceKind; limit?: number }) {
  const theme = useAppTheme();
  const { t, locale } = useI18n();
  const { usage } = useUiState();
  const now = useUsageNow();
  const [width, setWidth] = useState(0);
  const days = usageWeek(usage, kind, now);
  const top = Math.max(limit ?? 0, ...days.map((d) => d.value), 1) * 1.18;
  const label = days
    .map(
      (d) =>
        `${usageDayLabel(d.day, d.today, locale)} ${usageAmount(kind, Math.round(d.value), locale)}`,
    )
    .join("，");
  const limitY = limit === undefined || limit <= 0 ? undefined : CHART_HEIGHT * (1 - limit / top);
  return (
    <View
      testID={`usage-week-${kind}`}
      accessible
      accessibilityLabel={`${t("usage.view.week")}：${label}`}>
      <View
        style={{ height: CHART_HEIGHT }}
        onLayout={(e: LayoutChangeEvent) => {
          setWidth(e.nativeEvent.layout.width);
        }}>
        {limitY !== undefined && (
          <>
            <Svg
              width={width}
              height={2}
              style={{ position: "absolute", left: 0, top: limitY }}
              pointerEvents="none">
              <Line
                x1={0}
                y1={1}
                x2={width}
                y2={1}
                stroke={theme.voltip.subtle}
                strokeOpacity={0.6}
                strokeWidth={1}
                strokeDasharray="4 3"
              />
            </Svg>
            <Text
              style={[
                styles.limitLabel,
                { top: Math.max(0, limitY - 15), color: theme.voltip.subtle },
              ]}>
              {limit}
            </Text>
          </>
        )}
        <View
          style={[styles.bars, { borderBottomColor: theme.colors.outlineVariant }]}
          pointerEvents="none">
          {days.map((d) => {
            const full = limit !== undefined && limit > 0 && d.value >= limit;
            const near = limit !== undefined && limit > 0 && d.value >= limit * 0.8;
            const fill = full
              ? theme.voltip.danger
              : near
                ? theme.voltip.warning
                : d.today
                  ? theme.colors.primary
                  : rgba(theme.colors.primary, 0.35);
            return (
              <View key={d.day} style={styles.barCell}>
                <View
                  testID="usage-week-bar"
                  style={{
                    width: BAR_WIDTH,
                    maxWidth: "70%",
                    height: Math.max(d.value > 0 ? 3 : 2, (d.value / top) * CHART_HEIGHT),
                    borderTopLeftRadius: 5,
                    borderTopRightRadius: 5,
                    backgroundColor: fill,
                  }}
                />
              </View>
            );
          })}
        </View>
      </View>
      <View style={styles.dayRow}>
        {days.map((d) => (
          <Text
            key={d.day}
            style={[
              styles.dayLabel,
              {
                color: d.today ? theme.colors.onSurface : theme.voltip.subtle,
                fontWeight: d.today ? "500" : "400",
              },
            ]}>
            {usageDayLabel(d.day, d.today, locale)}
          </Text>
        ))}
      </View>
    </View>
  );
}

/** Other providers this month, as this phone recorded them (board 16b): each provider and service
 *  with its amount, and where its remaining quota can be seen. */
export function OthersCard() {
  const theme = useAppTheme();
  const { t, locale } = useI18n();
  const { usage } = useUiState();
  // The phone runs no models of its own.
  const rows = usage.others.filter((o) => o.provider !== "local");
  return (
    <View
      testID="usage-others"
      style={[
        styles.card,
        {
          backgroundColor: theme.voltip.card,
          borderColor: theme.voltip.cardEdge,
          borderRadius: theme.voltip.radius.card,
        },
      ]}>
      <Text style={[styles.cardTitle, { color: theme.colors.onSurface }]}>
        {t("usage.view.others")}
      </Text>
      {rows.length === 0 ? (
        <Text style={[styles.small, { color: theme.voltip.subtle }]}>
          {t("usage.view.othersEmpty")}
        </Text>
      ) : (
        rows.map((o) => (
          <View
            key={`${o.provider}:${o.service}:${o.model ?? ""}`}
            style={styles.otherRow}
            testID="usage-other">
            <Text style={[styles.otherName, { color: theme.colors.onSurface }]} numberOfLines={2}>
              {`${t(`engines.provider.${o.provider}`)} · ${t(`usage.view.othersUse.${o.service}`, {
                amount: providerUseAmount(o, locale),
              })}`}
            </Text>
            <Text style={[styles.otherQuota, { color: theme.voltip.subtle }]} numberOfLines={2}>
              {o.provider === "aliyun"
                ? t("usage.view.othersQuota.aliyun")
                : t("usage.view.othersQuota.other")}
            </Text>
          </View>
        ))
      )}
      <Text style={[styles.small, { color: theme.voltip.subtle }]}>
        {t("mobile.usage.othersNote")}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  track: { overflow: "hidden", flexShrink: 0 },
  talkLine: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    marginTop: 8,
    paddingTop: 12,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  talkText: { flex: 1, fontSize: 12, lineHeight: 16 },
  out: { alignItems: "center", gap: 6 },
  outTitle: { fontSize: 17, lineHeight: 23, fontWeight: "500", textAlign: "center" },
  outBody: { fontSize: 12, lineHeight: 18, textAlign: "center" },
  outActions: {
    flexDirection: "row",
    flexWrap: "wrap",
    justifyContent: "center",
    gap: 12,
    marginTop: 8,
  },
  noticeActions: { flexDirection: "row", flexWrap: "wrap", gap: 8, marginTop: 4 },
  card: { borderWidth: 1, padding: 16, gap: 10 },
  cardHead: { flexDirection: "row", alignItems: "center", gap: 8 },
  cardTitle: { flex: 1, fontSize: 15, lineHeight: 21, fontWeight: "500" },
  cardStatus: { fontSize: 13, lineHeight: 18, fontWeight: "500" },
  numbers: { flexDirection: "row", alignItems: "baseline" },
  big: { fontSize: 28, lineHeight: 34, fontWeight: "600" },
  of: { fontSize: 14, lineHeight: 20 },
  small: { fontSize: 12, lineHeight: 18 },
  unknown: { flexDirection: "row", alignItems: "center", gap: 8 },
  week: { gap: 6, marginTop: 4 },
  weekTitle: { fontSize: 12, lineHeight: 16, fontWeight: "500" },
  bars: {
    ...StyleSheet.absoluteFill,
    flexDirection: "row",
    alignItems: "flex-end",
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  barCell: { flex: 1, alignItems: "center", justifyContent: "flex-end" },
  limitLabel: { position: "absolute", right: 0, fontSize: 10, lineHeight: 13 },
  dayRow: { flexDirection: "row" },
  dayLabel: { flex: 1, textAlign: "center", fontSize: 11, lineHeight: 15 },
  otherRow: { flexDirection: "row", alignItems: "flex-start", gap: 12 },
  otherName: { flex: 1, fontSize: 13, lineHeight: 18 },
  otherQuota: { flexShrink: 1, maxWidth: "50%", fontSize: 12, lineHeight: 17, textAlign: "right" },
});
