// The built-in service's daily usage as the interfaces show it (docs/dictation.md §27): each
// service's state against its limit, the amounts in its unit (minutes of audio, clean-ups), and
// when the day starts again. The numbers are the service's own (`UsageView.quota`, from the reports
// its answers carry); the days before today and what today was made of are this device's record.

import { durationParts } from "./labels";
import { shortClockLabel, startOfDay } from "./history-stats";
import { type Locale, DEFAULT_LOCALE, translate } from "./i18n";
import type {
  EngineStatus,
  FallbackModelStatus,
  FallbackStatus,
  ProviderId,
  ProviderUse,
  QuotaReport,
  ServiceKind,
  UsageNotice,
  UsageOrigin,
  UsagePurpose,
  UsageView,
} from "./schema";

/** At this share of the day's limit a service turns amber. */
export const USAGE_NEAR_SHARE = 0.8;

/** How a service stands today: under 80 % of its limit, from 80 % on, at the limit, in the counting
 *  period without a limit, unknown (the service could not be asked for a minute), not reported yet. */
export type UsageTone = "ok" | "near" | "out" | "unlimited" | "unknown" | "pending";

/** One service's usage today, in its display unit: whole minutes of audio for recognition, requests
 *  for the clean-up (polish and voice edits). */
export interface ServiceUsage {
  kind: ServiceKind;
  tone: UsageTone;
  /** Used today, rounded down. */
  used: number;
  /** The day's limit; absent in the counting period and before the first report. */
  limit?: number;
  /** What is left (rounded down; 0 at the limit); absent without a limit. */
  left?: number;
  /** Recognition: some audio is left, but less than a minute. */
  leftUnderOne: boolean;
  /** 0..1 of the limit, for the bars; 0 without one. */
  share: number;
  /** When the day starts again, Unix milliseconds; absent before the first report. */
  resetAtMs?: number;
}

/** The report as it stands at `nowMs`: past its reset, a new day with nothing used yet (the core
 *  does the same; this covers the time until its next report). */
export function currentQuota(
  quota: QuotaReport | undefined,
  nowMs: number,
): QuotaReport | undefined {
  if (quota === undefined || nowMs < quota.reset_at * 1000) return quota;
  return {
    ...quota,
    asr: { ...quota.asr, used_seconds: 0 },
    refine: { ...quota.refine, used: 0 },
    reset_at: nextBeijingMidnightMs(nowMs) / 1000,
  };
}

const DAY_MS = 86_400_000;
const BEIJING_OFFSET_MS = 8 * 3_600_000;

/** The next midnight in Beijing after `ms`: when the built-in service's day starts again. */
export function nextBeijingMidnightMs(ms: number): number {
  return (Math.floor((ms + BEIJING_OFFSET_MS) / DAY_MS) + 1) * DAY_MS - BEIJING_OFFSET_MS;
}

/** `kind`'s usage today at `nowMs`. */
export function serviceUsage(view: UsageView, kind: ServiceKind, nowMs: number): ServiceUsage {
  const quota = currentQuota(view.quota, nowMs);
  const base = { kind, used: 0, leftUnderOne: false, share: 0 };
  if (view.unavailable) return { ...base, tone: "unknown" };
  if (quota === undefined) return { ...base, tone: "pending" };
  const resetAtMs = quota.reset_at * 1000;
  const [usedExact, limitExact] =
    kind === "asr"
      ? [quota.asr.used_seconds, quota.asr.limit_seconds]
      : [quota.refine.used, quota.refine.limit];
  const unit = kind === "asr" ? 60 : 1;
  const used = Math.floor(usedExact / unit);
  if (quota.mode === "count" || limitExact === null) {
    return { ...base, tone: "unlimited", used, resetAtMs };
  }
  const leftExact = Math.max(0, limitExact - usedExact);
  const share = limitExact > 0 ? Math.min(1, usedExact / limitExact) : 1;
  const tone: UsageTone =
    usedExact >= limitExact ? "out" : share >= USAGE_NEAR_SHARE ? "near" : "ok";
  return {
    kind,
    tone,
    used,
    limit: Math.round(limitExact / unit),
    left: Math.floor(leftExact / unit),
    leftUnderOne: kind === "asr" && leftExact > 0 && leftExact < 60,
    share,
    resetAtMs,
  };
}

/** The built-in service is the provider `kind` uses now. */
export function usesBuiltin(engines: EngineStatus, kind: ServiceKind): boolean {
  return kind === "asr" ? engines.asr_provider === "builtin" : engines.llm_provider === "builtin";
}

const TONE_RANK: Readonly<Record<UsageTone, number>> = {
  out: 5,
  near: 4,
  unknown: 3,
  ok: 2,
  unlimited: 1,
  pending: 0,
};

/** The tighter of two services: what colours the card's icon and its one-line form. */
export function tighter(a: ServiceUsage, b: ServiceUsage): ServiceUsage {
  if (TONE_RANK[b.tone] !== TONE_RANK[a.tone]) return TONE_RANK[b.tone] > TONE_RANK[a.tone] ? b : a;
  return b.share > a.share ? b : a;
}

/** An amount in the service's unit: `23 分钟`, `41 次` (`23 min`, `41 uses`). */
export function usageAmount(kind: ServiceKind, n: number, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, `usage.amount.${kind}`, { n });
}

/** `23 / 60 分钟`, `41 / 100 次`. */
export function usageOf(usage: ServiceUsage, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, "usage.of", {
    used: usage.used,
    limit: usageAmount(usage.kind, usage.limit ?? 0, locale),
  });
}

/** `剩余 8 分钟`, or `剩余不到 1 分钟`. */
export function usageLeft(usage: ServiceUsage, locale: Locale = DEFAULT_LOCALE): string {
  if (usage.leftUnderOne) return translate(locale, "usage.leftUnderOne");
  return translate(locale, "usage.left", {
    amount: usageAmount(usage.kind, usage.left ?? 0, locale),
  });
}

/** The value a service shows on its row: the numbers, what is left from 80 % on, 已用完 at the
 *  limit, the amount alone in the counting period. */
export function usageValue(usage: ServiceUsage, locale: Locale = DEFAULT_LOCALE): string {
  switch (usage.tone) {
    case "out":
      return translate(locale, "usage.usedUp");
    case "near":
      return usageLeft(usage, locale);
    case "unlimited":
      return usageAmount(usage.kind, usage.used, locale);
    case "unknown":
      return translate(locale, "usage.unavailable");
    case "pending":
      return translate(locale, "common.none");
    case "ok":
      return usageOf(usage, locale);
  }
}

/** The one-line form under a model (home, the provider cards, the phone's talk page). */
export function usageInline(
  usage: ServiceUsage,
  nowMs: number,
  locale: Locale = DEFAULT_LOCALE,
): string {
  switch (usage.tone) {
    case "out":
      return translate(locale, "usage.inline.out", {
        reset: resetLabel(usage.resetAtMs ?? nowMs, nowMs, locale),
      });
    case "near":
      return usage.leftUnderOne
        ? translate(locale, "usage.inline.underOne")
        : translate(locale, "usage.inline.left", {
            amount: usageAmount(usage.kind, usage.left ?? 0, locale),
          });
    case "unlimited":
      return translate(locale, "usage.inline.unlimited", {
        amount: usageAmount(usage.kind, usage.used, locale),
      });
    case "unknown":
      return translate(locale, "usage.unavailableBody");
    case "pending":
      return translate(locale, "usage.pending");
    case "ok":
      return translate(locale, "usage.inline.of", {
        used: usage.used,
        limit: usageAmount(usage.kind, usage.limit ?? 0, locale),
      });
  }
}

/** Both services in one line, as the phone's 设置 › 用量 row says it (Penpot 「用量与额度」 board
 *  16a): `识别 23 / 60 分钟 · 润色 41 / 100 次`, what is left from 80 % on, 已用完 at the limit, the
 *  amounts with 暂不限量 in the counting period; one line for the day when it cannot be told. */
export function usageSummary(
  view: UsageView,
  nowMs: number,
  locale: Locale = DEFAULT_LOCALE,
): string {
  const asr = serviceUsage(view, "asr", nowMs);
  if (asr.tone === "unknown") return translate(locale, "usage.unavailableBody");
  if (asr.tone === "pending") return translate(locale, "usage.pending");
  const llm = serviceUsage(view, "llm", nowMs);
  const part = (usage: ServiceUsage) =>
    translate(
      locale,
      usage.tone === "ok" || usage.tone === "unlimited"
        ? "usage.card.compactNumbers"
        : "usage.card.compactValue",
      { service: translate(locale, `usage.short.${usage.kind}`), value: usageValue(usage, locale) },
    );
  const line = `${part(asr)} · ${part(llm)}`;
  return asr.tone === "unlimited" ? `${line} · ${translate(locale, "usage.unlimited")}` : line;
}

/** `00:00` or `16:00`: the local clock time of a reset. */
export function resetClock(resetAtMs: number): string {
  return shortClockLabel(resetAtMs);
}

/** `明天 00:00 重置`, `今天 16:00 重置` (the local day and clock of the reset). */
export function resetLabel(
  resetAtMs: number,
  nowMs: number,
  locale: Locale = DEFAULT_LOCALE,
): string {
  const days = Math.round((startOfDay(resetAtMs) - startOfDay(nowMs)) / DAY_MS);
  const time = resetClock(resetAtMs);
  if (days <= 0) return translate(locale, "usage.reset.today", { time });
  if (days === 1) return translate(locale, "usage.reset.tomorrow", { time });
  return translate(locale, "usage.reset.later", { time, n: days });
}

/** `还有 6 小时 12 分` (to the minute; `还有不到 1 分钟` in the last one). */
export function resetCountdown(
  resetAtMs: number,
  nowMs: number,
  locale: Locale = DEFAULT_LOCALE,
): string {
  const ms = Math.max(0, resetAtMs - nowMs);
  if (ms < 60_000) return translate(locale, "usage.countdownUnderOne");
  const minutes = Math.floor(ms / 60_000);
  const parts = durationParts(minutes * 60_000, locale);
  return translate(locale, "usage.countdown", {
    duration: parts.map((p) => `${p.value} ${p.unit}`).join(" "),
  });
}

/** `00:00` when this machine keeps Beijing time; the local time of Beijing's midnight otherwise. */
export function dailyResetLine(nowMs: number, locale: Locale = DEFAULT_LOCALE): string {
  const midnight = nextBeijingMidnightMs(nowMs);
  const time = resetClock(midnight);
  return time === "00:00"
    ? translate(locale, "usage.resetDaily")
    : translate(locale, "usage.resetDailyLocal", { time });
}

/** Where today's built-in recognition came from, as a row names it. */
export function usageOriginName(origin: UsageOrigin, locale: Locale = DEFAULT_LOCALE): string {
  return origin.kind === "phone" ? origin.name : translate(locale, `usage.origin.${origin.kind}`);
}

/** The line after a row's name: what the origin is. */
export function usageOriginNote(origin: UsageOrigin, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, `usage.originNote.${origin.kind}`);
}

/** A clean-up purpose's name and what it is. */
export function usagePurposeName(purpose: UsagePurpose, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, purpose === "edit" ? "usage.purpose.edit" : "usage.purpose.refine");
}

export function usagePurposeNote(purpose: UsagePurpose, locale: Locale = DEFAULT_LOCALE): string {
  return translate(
    locale,
    purpose === "edit" ? "usage.purposeNote.edit" : "usage.purposeNote.refine",
  );
}

/** Seconds of audio as the usage page names them: `38 分钟` under an hour, `2 小时 10 分` from
 *  then on, `不到 1 分钟` for a few seconds. */
export function audioAmount(seconds: number, locale: Locale = DEFAULT_LOCALE): string {
  if (seconds > 0 && seconds < 60) return translate(locale, "usage.underOneMinute");
  if (seconds < 3600) return usageAmount("asr", Math.floor(seconds / 60), locale);
  return durationParts(Math.floor(seconds / 60) * 60_000, locale)
    .map((p) => `${p.value} ${p.unit}`)
    .join(" ");
}

/** A provider's use of one service: recognition as a span of audio, the clean-up as requests. */
export function providerUseAmount(use: ProviderUse, locale: Locale = DEFAULT_LOCALE): string {
  return use.service === "llm"
    ? usageAmount("llm", use.amount, locale)
    : audioAmount(use.amount, locale);
}

/** Today's use of the other providers of `kind`: the total and the provider used most. */
export function othersToday(
  view: UsageView,
  kind: ServiceKind,
): { amount: number; provider?: ProviderId } {
  const rows = view.others_today.filter((o) => o.service === kind);
  const amount = rows.reduce((sum, o) => sum + o.amount, 0);
  const top = rows[0];
  return top === undefined ? { amount } : { amount, provider: top.provider };
}

/** The notice the home page shows for `kind` today (or none): at 80 % and at the limit, while the
 *  built-in service is the one in use and limits apply, unless closed today. */
export function usageNotice(
  usage: ServiceUsage,
  view: UsageView,
  inUse: boolean,
): UsageNotice | undefined {
  if (!inUse || !view.builtin) return undefined;
  const prefix = usage.kind === "asr" ? "asr" : "refine";
  const notice: UsageNotice | undefined =
    usage.tone === "out" ? `${prefix}_out` : usage.tone === "near" ? `${prefix}_near` : undefined;
  if (notice === undefined || view.dismissed.includes(notice)) return undefined;
  return notice;
}

/** The last seven days of `kind` for the chart, oldest first: whole minutes or clean-ups; today
 *  is the service's own number when it reported one. */
export function usageWeek(
  view: UsageView,
  kind: ServiceKind,
  nowMs: number,
): { day: string; value: number; today: boolean }[] {
  const usage = serviceUsage(view, kind, nowMs);
  const reported = usage.tone !== "pending" && usage.tone !== "unknown";
  return view.days.map((d, i) => {
    const today = i === view.days.length - 1;
    const own = kind === "asr" ? d.asr_seconds / 60 : d.refine;
    return {
      day: d.day,
      value: today && reported ? Math.max(usage.used, Math.floor(own)) : own,
      today,
    };
  });
}

/** `周三` / `Wed` for a chart's day (`YYYY-MM-DD`, Beijing time); the last is 今天. */
export function usageDayLabel(
  day: string,
  today: boolean,
  locale: Locale = DEFAULT_LOCALE,
): string {
  if (today) return translate(locale, "time.today");
  const [y, m, d] = day.split("-").map((part) => Number.parseInt(part, 10));
  const date = new Date(Date.UTC(y ?? 1970, (m ?? 1) - 1, d ?? 1, 12));
  return new Intl.DateTimeFormat(locale, { weekday: "short", timeZone: "UTC" }).format(date);
}

/** The fallback model that recognises or cleans up while the built-in service's day is used up
 *  (docs/dictation.md §3.5): the first one that can run and is not waiting for its own quota; none
 *  when the chain is off. */
export function usableFallback(
  status: FallbackStatus | undefined,
  nowMs: number,
): FallbackModelStatus | undefined {
  if (status === undefined || !status.in_use) return undefined;
  return status.models.find(
    (m) => m.issue === undefined && m.skip === undefined && (m.retry_at_ms ?? 0) <= nowMs,
  );
}
