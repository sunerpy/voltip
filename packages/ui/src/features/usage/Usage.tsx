import {
  type ServiceKind,
  type ServiceUsage,
  type UsageTone,
  resetCountdown,
  resetLabel,
  serviceUsage,
  usageAmount,
  usageInline,
  usageLeft,
  usageOf,
  usageOriginName,
  usageOriginNote,
  usagePurposeName,
  usagePurposeNote,
  usageValue,
  usageWeek,
  usageDayLabel,
  audioAmount,
} from "@voltip/shared";
import { useBackend, useUiState } from "../../backend/BackendProvider";
import { Icon, type IconName } from "../../components/Icon";
import { cx } from "../../cx";
import { useNow } from "../../hooks/useNow";
import { useI18n } from "../../i18n/I18nProvider";

/** The bar colour of each state (Penpot 「用量与额度」 board 11): the accent, amber from 80 %, red
 *  at the limit; nothing while the numbers are unknown. */
const BAR_TONE: Readonly<Record<UsageTone, string>> = {
  ok: "bg-accent",
  unlimited: "bg-accent",
  near: "bg-warning-fill",
  out: "bg-danger",
  unknown: "bg-transparent",
  pending: "bg-transparent",
};

/** The text colour of a state's status (`剩余 8 分钟`, `已用完`). */
export const USAGE_TEXT_TONE: Readonly<Record<UsageTone, string>> = {
  ok: "text-fg-muted",
  unlimited: "text-fg-muted",
  near: "text-warning",
  out: "text-danger",
  unknown: "text-fg-subtle",
  pending: "text-fg-subtle",
};

/** The icon of each service. */
export const USAGE_ICON: Readonly<Record<ServiceKind, IconName>> = { asr: "wave", llm: "wand" };

/** The current time in milliseconds, refreshed every 30 s (`useNow`). */
export function useUsageNow(): number {
  return useNow() * 1000;
}

/** `kind`'s usage today from `UiState.usage`. */
export function useServiceUsage(kind: ServiceKind): ServiceUsage {
  const { usage } = useUiState();
  return serviceUsage(usage, kind, useUsageNow());
}

export interface UsageBarProps {
  share: number;
  tone: UsageTone;
  /** Height in px. */
  size?: 4 | 6 | 8;
  /** The bar's name (the service and its numbers). */
  label: string;
  /** A fixed width in px; the bar fills its container otherwise. */
  width?: number;
  className?: string;
}

/** A day's use against its limit: a rounded track filled to `share` (a little use still shows as
 *  a dot). */
export function UsageBar({ share, tone, size = 6, label, width, className }: UsageBarProps) {
  const pct = Math.round(Math.min(1, Math.max(0, share)) * 100);
  return (
    <div
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct}
      data-tone={tone}
      className={cx(
        "relative overflow-hidden rounded-pill bg-led-off",
        width === undefined && "w-full",
        className,
      )}
      style={{ height: size, ...(width === undefined ? {} : { width }) }}>
      {share > 0 && (
        <span
          className={cx("absolute inset-y-0 left-0 rounded-pill", BAR_TONE[tone])}
          style={{ width: `max(${pct}%, ${size}px)` }}
        />
      )}
    </div>
  );
}

export interface UsageInlineProps {
  usage: ServiceUsage;
  /** Text before the usage (`内置服务 · `). */
  prefix?: string;
  /** The bar's width in px; 0 for none. */
  barWidth?: number;
  className?: string;
  "data-testid"?: string;
}

/** One line under a model (board 11 「单行用量」): today's numbers and a short bar; amber and red
 *  from 80 % and at the limit. */
export function UsageInline({
  usage,
  prefix,
  barWidth = 56,
  className,
  "data-testid": testId,
}: UsageInlineProps) {
  const { t, locale } = useI18n();
  const now = useUsageNow();
  const text = usageInline(usage, now, locale);
  const bar = barWidth > 0 && (usage.tone === "ok" || usage.tone === "near");
  return (
    <span
      className={cx("flex min-w-0 items-center gap-2.5", className)}
      data-testid={testId}
      data-tone={usage.tone}>
      <span
        className={cx(
          "min-w-0 truncate text-[12px]",
          usage.tone === "near" || usage.tone === "out" ? "font-medium" : undefined,
          usage.tone === "ok" || usage.tone === "unlimited"
            ? "text-fg-muted"
            : USAGE_TEXT_TONE[usage.tone],
        )}
        title={prefix === undefined ? text : `${prefix}${text}`}>
        {prefix}
        {text}
      </span>
      {bar && (
        <UsageBar
          share={usage.share}
          tone={usage.tone}
          size={6}
          width={barWidth}
          label={`${t(`usage.service.${usage.kind}`)} · ${text}`}
          className="shrink-0"
        />
      )}
    </span>
  );
}

export interface UsageMeterProps {
  usage: ServiceUsage;
  /** Shown while the numbers cannot be fetched: asks the service again. */
  onRetry?: () => void;
  className?: string;
  "data-testid"?: string;
}

/** One service's day as the drawers and the components board draw it: the name with the status on
 *  the right, an 8 px bar, the numbers and when the day starts again. */
export function UsageMeter({ usage, onRetry, className, "data-testid": testId }: UsageMeterProps) {
  const { t, locale } = useI18n();
  const now = useUsageNow();
  const title = t(`usage.service.${usage.kind}`);
  const reset =
    usage.resetAtMs === undefined
      ? undefined
      : `${resetLabel(usage.resetAtMs, now, locale)} · ${resetCountdown(usage.resetAtMs, now, locale)}`;
  // What is left, from the first minute on; the day's state otherwise.
  const status =
    usage.tone === "out"
      ? t("usage.usedUpToday")
      : usage.tone === "unlimited"
        ? t("usage.unlimited")
        : usage.tone === "ok" || usage.tone === "near"
          ? usageLeft(usage, locale)
          : usageValue(usage, locale);
  return (
    <div
      className={cx("flex flex-col gap-2", className)}
      data-testid={testId}
      data-tone={usage.tone}>
      <div className="flex min-w-0 items-center gap-2">
        <Icon
          name={USAGE_ICON[usage.kind]}
          size={16}
          className={cx("shrink-0", usage.tone === "out" ? "text-danger" : "text-fg-muted")}
        />
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-fg">{title}</span>
        <span
          className={cx(
            "flex shrink-0 items-center gap-1.5 text-[12px] font-medium",
            USAGE_TEXT_TONE[usage.tone],
            usage.tone === "ok" && "text-fg",
          )}
          data-testid="usage-meter-status">
          {(usage.tone === "near" || usage.tone === "out") && (
            <span
              aria-hidden
              className={cx(
                "size-1.5 rounded-full",
                usage.tone === "out" ? "bg-danger" : "bg-warning-fill",
              )}
            />
          )}
          {status}
        </span>
      </div>
      {usage.tone === "unlimited" ? (
        <div className="text-[15px] font-medium text-fg">
          {t("usage.todayUsed", { amount: usageAmount(usage.kind, usage.used, locale) })}
        </div>
      ) : (
        <UsageBar
          share={usage.tone === "out" ? 1 : usage.share}
          tone={usage.tone}
          size={8}
          label={`${title} · ${status}`}
        />
      )}
      <div className="flex min-w-0 items-center justify-between gap-3 text-[12px]">
        {usage.tone === "unknown" || usage.tone === "pending" ? (
          <>
            <span className="truncate text-fg-subtle">
              {usage.tone === "unknown" ? t("usage.unavailableBody") : t("usage.pending")}
            </span>
            {usage.tone === "unknown" && onRetry !== undefined && (
              <button
                type="button"
                onClick={onRetry}
                data-testid="usage-retry"
                className="inline-flex shrink-0 items-center gap-1 font-medium text-accent-text hover:text-accent-text-hover">
                <Icon name="refresh" size={13} />
                {t("common.retry")}
              </button>
            )}
          </>
        ) : (
          <>
            <span className="shrink-0 text-fg-muted">
              {usage.tone === "unlimited"
                ? t("usage.unlimitedShort")
                : t("usage.used", { of: usageOf(usage, locale) })}
            </span>
            {reset !== undefined && (
              <span className="min-w-0 truncate text-right text-fg-subtle" title={reset}>
                {reset}
              </span>
            )}
          </>
        )}
      </div>
    </div>
  );
}

export interface UsageWeekChartProps {
  kind: ServiceKind;
  /** The day's limit in the chart's unit; no dashed line without one. */
  limit?: number;
  height?: number;
  className?: string;
}

/** The last seven days of one service (board 14): bars against a dashed line at the day's limit,
 *  amber from 80 %, red at the limit, today in the accent. */
export function UsageWeekChart({ kind, limit, height = 112, className }: UsageWeekChartProps) {
  const { t, locale } = useI18n();
  const { usage } = useUiState();
  const now = useUsageNow();
  const days = usageWeek(usage, kind, now);
  // Some room over the tallest bar or the limit, for the limit's label.
  const top = Math.max(limit ?? 0, ...days.map((d) => d.value), 1) * 1.18;
  const label = days
    .map(
      (d) =>
        `${usageDayLabel(d.day, d.today, locale)} ${usageAmount(kind, Math.round(d.value), locale)}`,
    )
    .join("，");
  return (
    <figure
      className={cx("flex flex-col gap-1.5", className)}
      aria-label={`${t("usage.view.week")}：${label}`}
      data-testid={`usage-week-${kind}`}>
      <div className="relative" style={{ height }}>
        {limit !== undefined && limit > 0 && (
          <>
            <span
              aria-hidden
              className="absolute inset-x-0 border-t border-dashed border-fg-subtle/50"
              style={{ bottom: `${(limit / top) * 100}%` }}
            />
            <span
              aria-hidden
              className="mono absolute right-0 text-[10px] text-fg-subtle"
              style={{ bottom: `calc(${(limit / top) * 100}% + 2px)` }}>
              {limit}
            </span>
          </>
        )}
        <div
          aria-hidden
          className="absolute inset-0 flex items-end justify-around border-b border-border">
          {days.map((d) => {
            const share = d.value / top;
            const full = limit !== undefined && limit > 0 && d.value >= limit;
            const near = limit !== undefined && limit > 0 && d.value >= limit * 0.8;
            return (
              <span
                key={d.day}
                data-day={d.day}
                data-testid="usage-week-bar"
                title={`${usageDayLabel(d.day, d.today, locale)} · ${usageAmount(kind, Math.round(d.value), locale)}`}
                className={cx(
                  "w-[min(28px,46%)] max-w-7 rounded-t-[5px]",
                  full
                    ? "bg-danger"
                    : near
                      ? "bg-warning-fill"
                      : d.today
                        ? "bg-accent"
                        : "bg-accent/35",
                )}
                style={{ height: `${Math.max(d.value > 0 ? 3 : 2, share * 100)}%` }}
              />
            );
          })}
        </div>
      </div>
      <div aria-hidden className="flex justify-around">
        {days.map((d) => (
          <span
            key={d.day}
            className={cx(
              "w-10 text-center text-[11px]",
              d.today ? "font-medium text-fg" : "text-fg-subtle",
            )}>
            {usageDayLabel(d.day, d.today, locale)}
          </span>
        ))}
      </div>
    </figure>
  );
}

export interface ShareRow {
  key: string;
  name: string;
  note?: string;
  value: string;
  share: number;
  /** A softer bar (the second purpose). */
  soft?: boolean;
}

/** Rows of a breakdown (board 14 今日明细): a name, what it is, the amount, and its share as a bar. */
export function UsageShares({
  rows,
  className,
}: {
  rows: readonly ShareRow[];
  className?: string;
}) {
  return (
    <ul className={cx("flex flex-col gap-3.5", className)}>
      {rows.map((row) => (
        <li key={row.key} className="flex flex-col gap-1.5" data-testid="usage-share">
          <div className="flex min-w-0 items-baseline gap-2">
            <span
              className="shrink-0 text-[13px] font-medium text-fg"
              {...(row.key.startsWith("phone:") ? { "data-user-text": "" } : {})}>
              {row.name}
            </span>
            {row.note !== undefined && (
              <span className="min-w-0 flex-1 truncate text-[12px] text-fg-subtle">{row.note}</span>
            )}
            <span className="ml-auto shrink-0 text-[13px] font-medium text-fg-muted">
              {row.value}
            </span>
          </div>
          <div aria-hidden className="relative h-1.5 overflow-hidden rounded-pill bg-led-off">
            <span
              className={cx(
                "absolute inset-y-0 left-0 rounded-pill",
                row.soft ? "bg-accent/35" : "bg-accent",
              )}
              style={{ width: `${Math.round(Math.min(1, row.share) * 100)}%` }}
            />
          </div>
        </li>
      ))}
    </ul>
  );
}

/** Today's built-in recognition by origin, or clean-up by purpose, as share rows. */
export function useUsageBreakdown(kind: ServiceKind): { rows: ShareRow[]; total: string } {
  const { locale } = useI18n();
  const { usage } = useUiState();
  if (kind === "asr") {
    const total = usage.asr_origins.reduce((sum, o) => sum + o.seconds, 0);
    return {
      total: audioAmount(total, locale),
      rows: usage.asr_origins
        .filter((o) => o.seconds > 0)
        .map((o) => ({
          key: o.origin.kind === "phone" ? `phone:${o.origin.name}` : o.origin.kind,
          name: usageOriginName(o.origin, locale),
          note: usageOriginNote(o.origin, locale),
          value: audioAmount(o.seconds, locale),
          share: total > 0 ? o.seconds / total : 0,
        })),
    };
  }
  const total = usage.refine_purposes.reduce((sum, p) => sum + p.count, 0);
  return {
    total: usageAmount("llm", total, locale),
    rows: usage.refine_purposes
      .filter((p) => p.count > 0)
      .map((p, i) => ({
        key: p.purpose,
        name: usagePurposeName(p.purpose, locale),
        note: usagePurposeNote(p.purpose, locale),
        value: usageAmount("llm", p.count, locale),
        share: total > 0 ? p.count / total : 0,
        soft: i > 0,
      })),
  };
}

/** The built-in service's usage in its drawer on the model pages (board 12): today against the
 *  limit, how it is counted, the last seven days and what today was made of. */
export function BuiltinUsage({ kind }: { kind: ServiceKind }) {
  const { t, locale } = useI18n();
  const { backend } = useBackend();
  const { usage: view } = useUiState();
  const usage = useServiceUsage(kind);
  const now = useUsageNow();
  const breakdown = useUsageBreakdown(kind);
  if (!view.builtin) return null;
  const reset = resetLabel(usage.resetAtMs ?? now, now, locale);
  return (
    <section className="flex flex-col gap-4" data-testid={`builtin-usage-${kind}`}>
      <h3 className="eyebrow">{t("usage.drawer.title")}</h3>
      <UsageMeter
        usage={usage}
        onRetry={() => {
          void backend.invoke("usage_refresh");
        }}
      />
      <p className="text-[12px] leading-5 text-fg-muted">
        {t(`usage.drawer.body.${kind}`, { reset })}
      </p>
      <div className="flex flex-col gap-2 border-t border-border pt-4">
        <div className="flex items-baseline justify-between gap-3">
          <span className="text-[12px] font-medium text-fg-muted">{t("usage.view.week")}</span>
          <span className="text-[11px] text-fg-subtle">
            {usage.limit === undefined
              ? t(`usage.view.weekUnitNoLimit.${kind}`)
              : t(`usage.view.weekUnit.${kind}`)}
          </span>
        </div>
        <UsageWeekChart
          kind={kind}
          {...(usage.limit === undefined ? {} : { limit: usage.limit })}
          height={96}
        />
      </div>
      <div className="flex flex-col gap-3 border-t border-border pt-4">
        <div className="flex items-baseline justify-between gap-3">
          <span className="text-[12px] font-medium text-fg-muted">
            {kind === "asr" ? t("usage.drawer.origins") : t("usage.drawer.purposes")}
          </span>
          <span className="text-[11px] text-fg-subtle">
            {t("usage.view.total", { amount: breakdown.total })}
          </span>
        </div>
        {breakdown.rows.length === 0 ? (
          <span className="text-[12px] text-fg-subtle">{t("usage.view.nothing")}</span>
        ) : (
          <UsageShares rows={breakdown.rows} />
        )}
      </div>
      <button
        type="button"
        data-testid="usage-rules"
        onClick={() => {
          backend.guideOpen("usage", locale).catch(() => undefined);
        }}
        className="inline-flex items-center gap-1.5 self-start text-[12px] font-medium text-accent-text hover:text-accent-text-hover">
        <Icon name="info" size={14} />
        {t("usage.drawer.rules")}
      </button>
    </section>
  );
}
