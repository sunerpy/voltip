import {
  type ProviderUse,
  type ServiceKind,
  type ServiceUsage,
  dailyResetLine,
  providerUseAmount,
  resetCountdown,
  resetLabel,
  shortModel,
  usageAmount,
  usageLeft,
  usageValue,
  usesBuiltin,
} from "@voltip/shared";
import {
  Card,
  Icon,
  ProviderMark,
  USAGE_ICON,
  USAGE_TEXT_TONE,
  UsageBar,
  UsageShares,
  UsageWeekChart,
  cx,
  hasProviderMark,
  useBackend,
  useI18n,
  useServiceUsage,
  useUiState,
  useUsageBreakdown,
  useUsageNow,
} from "@voltip/ui";
import { useEffect } from "react";

/** 用量 (Penpot 「用量与额度」 board 14; docs/dictation.md §27): the built-in recognition and
 *  clean-up today against their limits with the last seven days, what today was made of, and this
 *  month's other providers as this computer recorded them. Opened from the sidebar's card. The
 *  numbers are asked for again as the page opens. */
export function Usage() {
  const { t, locale } = useI18n();
  const { backend } = useBackend();
  const now = useUsageNow();
  useEffect(() => {
    void backend.invoke("usage_refresh");
  }, [backend]);
  return (
    <div className="mx-auto flex w-full max-w-[1100px] flex-col gap-4 p-6" data-testid="page-usage">
      <p className="text-[13px] text-fg-muted" data-testid="usage-intro">
        {t("usage.view.intro", { reset: dailyResetLine(now, locale) })}
      </p>
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <ServiceCard kind="asr" />
        <ServiceCard kind="llm" />
      </div>
      <TodayDetail />
      <OtherProviders />
    </div>
  );
}

/** One built-in service: today's number against the limit, when it resets, and the last week. */
function ServiceCard({ kind }: { kind: ServiceKind }) {
  const { t, locale } = useI18n();
  const { backend } = useBackend();
  const state = useUiState();
  const usage = useServiceUsage(kind);
  const now = useUsageNow();
  const inUse = usesBuiltin(state.engines, kind);
  const model = kind === "asr" ? state.engines.asr_model : state.engines.refine_model;
  const provider = kind === "asr" ? state.engines.asr_provider : state.engines.llm_provider;
  const title = t(`usage.service.${kind}`);
  return (
    <Card
      padding="none"
      className="flex flex-col gap-3 px-6 py-5"
      data-testid={`usage-card-${kind}`}
      data-tone={usage.tone}>
      <div className="flex min-w-0 items-center gap-2">
        <Icon
          name={USAGE_ICON[kind]}
          size={16}
          className={cx("shrink-0", usage.tone === "out" ? "text-danger" : "text-accent")}
        />
        <h2 className="min-w-0 flex-1 truncate text-[15px] font-medium text-fg">{title}</h2>
        {inUse && model.length > 0 && (
          <span className="mono shrink-0 truncate text-[11px] text-fg-subtle">
            {shortModel(model)}
          </span>
        )}
      </div>
      <Numbers usage={usage} />
      {usage.tone === "unknown" || usage.tone === "pending" ? (
        <div className="flex items-center justify-between gap-3 text-[12px]">
          <span className="text-fg-subtle">
            {usage.tone === "unknown" ? t("usage.unavailableBody") : t("usage.pending")}
          </span>
          {usage.tone === "unknown" && (
            <button
              type="button"
              onClick={() => {
                void backend.invoke("usage_refresh");
              }}
              data-testid={`usage-retry-${kind}`}
              className="inline-flex items-center gap-1 font-medium text-accent-text hover:text-accent-text-hover">
              <Icon name="refresh" size={13} />
              {t("common.retry")}
            </button>
          )}
        </div>
      ) : (
        <>
          {usage.tone !== "unlimited" && (
            <UsageBar
              share={usage.tone === "out" ? 1 : usage.share}
              tone={usage.tone}
              size={8}
              label={`${title} · ${usageValue(usage, locale)}`}
            />
          )}
          <div className="flex min-w-0 items-center justify-between gap-3 text-[12px] text-fg-subtle">
            <span className="truncate" data-testid={`usage-reset-${kind}`}>
              {usage.resetAtMs === undefined
                ? ""
                : `${resetLabel(usage.resetAtMs, now, locale)} · ${resetCountdown(usage.resetAtMs, now, locale)}`}
            </span>
            <span className="shrink-0">{t(`usage.view.basis.${kind}`)}</span>
          </div>
        </>
      )}
      {!inUse && provider !== undefined && provider !== null && (
        <p className="text-[12px] text-fg-muted" data-testid={`usage-not-in-use-${kind}`}>
          {t("usage.view.notInUse", {
            service: t(`usage.short.${kind}`),
            provider: t(`engines.provider.${provider}`),
          })}
        </p>
      )}
      <div className="mt-1 flex flex-col gap-2 border-t border-border pt-4">
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
        />
      </div>
    </Card>
  );
}

/** The big number: today's use over the limit, what is left on the right. */
function Numbers({ usage }: { usage: ServiceUsage }) {
  const { t, locale } = useI18n();
  const status =
    usage.tone === "out"
      ? t("usage.usedUpToday")
      : usage.tone === "unlimited"
        ? t("usage.unlimited")
        : usage.tone === "ok" || usage.tone === "near"
          ? usageLeft(usage, locale)
          : undefined;
  if (usage.tone === "unknown" || usage.tone === "pending") return null;
  return (
    <div
      className="flex items-end justify-between gap-3"
      data-testid={`usage-numbers-${usage.kind}`}>
      <span className="flex items-baseline gap-2">
        <span className="text-[32px] leading-none font-semibold text-fg">{usage.used}</span>
        <span className="text-[14px] text-fg-muted">
          {usage.limit === undefined
            ? t(`usage.unit.${usage.kind}`)
            : `/ ${usageAmount(usage.kind, usage.limit, locale)}`}
        </span>
      </span>
      {status !== undefined && (
        <span
          className={cx(
            "flex items-center gap-1.5 text-[13px] font-medium",
            usage.tone === "ok" ? "text-fg" : USAGE_TEXT_TONE[usage.tone],
          )}>
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
      )}
    </div>
  );
}

/** 今日明细: today's recognition by origin and clean-up by purpose. */
function TodayDetail() {
  const { t } = useI18n();
  const asr = useUsageBreakdown("asr");
  const llm = useUsageBreakdown("llm");
  return (
    <Card padding="none" className="flex flex-col gap-4 px-6 py-5" data-testid="usage-detail">
      <h2 className="text-[15px] font-medium text-fg">{t("usage.view.detail")}</h2>
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2 lg:gap-10">
        {(
          [
            ["asr", t("usage.view.origins"), asr],
            ["llm", t("usage.view.purposes"), llm],
          ] as const
        ).map(([kind, title, breakdown]) => (
          <section key={kind} className="flex flex-col gap-3" data-testid={`usage-detail-${kind}`}>
            <div className="flex items-baseline justify-between gap-3 text-[12px]">
              <span className="font-medium text-fg-muted">{title}</span>
              <span className="text-fg-subtle">
                {t("usage.view.total", { amount: breakdown.total })}
              </span>
            </div>
            {breakdown.rows.length === 0 ? (
              <span className="text-[12px] text-fg-subtle">{t("usage.view.nothing")}</span>
            ) : (
              <UsageShares rows={breakdown.rows} />
            )}
          </section>
        ))}
      </div>
    </Card>
  );
}

/** 其他服务商 · 本月: what this computer recorded of every other provider this month. */
function OtherProviders() {
  const { t } = useI18n();
  const { usage } = useUiState();
  return (
    <Card padding="none" className="flex flex-col gap-3 px-6 py-5" data-testid="usage-others">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <h2 className="text-[15px] font-medium text-fg">{t("usage.view.others")}</h2>
        <span className="text-[12px] text-fg-subtle">{t("usage.view.othersNote")}</span>
      </div>
      {usage.others.length === 0 ? (
        <p className="text-[12px] text-fg-subtle">{t("usage.view.othersEmpty")}</p>
      ) : (
        <table className="w-full table-fixed text-left text-[13px]">
          <thead>
            <tr className="border-b border-border text-[11px] text-fg-subtle">
              <th className="w-[44%] py-2 font-normal">{t("usage.view.othersHead.provider")}</th>
              <th className="w-[20%] py-2 font-normal">{t("usage.view.othersHead.used")}</th>
              <th className="py-2 font-normal">{t("usage.view.othersHead.quota")}</th>
            </tr>
          </thead>
          <tbody>
            {usage.others.map((use) => (
              <OtherRow key={`${use.provider}-${use.service}-${use.model ?? ""}`} use={use} />
            ))}
          </tbody>
        </table>
      )}
    </Card>
  );
}

function OtherRow({ use }: { use: ProviderUse }) {
  const { t, locale } = useI18n();
  const { backend } = useBackend();
  const { engines } = useUiState();
  const quotaConsole = engines.providers.find((p) => p.id === use.provider)?.quota === true;
  const quota =
    use.provider === "local"
      ? t("usage.view.othersQuota.local")
      : use.provider === "aliyun"
        ? t("usage.view.othersQuota.aliyun")
        : t("usage.view.othersQuota.other");
  return (
    <tr className="border-b border-border last:border-b-0" data-testid="usage-other">
      <td className="py-2.5">
        <span className="flex min-w-0 items-center gap-2.5">
          <span className="flex size-5 shrink-0 items-center justify-center text-fg-muted">
            {hasProviderMark(use.provider) ? (
              <ProviderMark id={use.provider} size={16} />
            ) : (
              <Icon name={use.provider === "local" ? "cpu" : "cloud"} size={15} />
            )}
          </span>
          <span className="shrink-0 font-medium text-fg">
            {t(`engines.provider.${use.provider}`)}
          </span>
          {use.model !== undefined && (
            <span className="mono min-w-0 truncate text-[11px] text-fg-subtle" title={use.model}>
              {use.model}
            </span>
          )}
        </span>
      </td>
      <td className="py-2.5 text-fg">
        {t(`usage.view.othersUse.${use.service}`, { amount: providerUseAmount(use, locale) })}
      </td>
      <td className="py-2.5">
        <span className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-[12px] text-fg-muted">
          <span className="min-w-0">{quota}</span>
          {quotaConsole && (
            <button
              type="button"
              onClick={() => {
                void backend.providerConsoleOpen(use.provider, "quota");
              }}
              className="inline-flex shrink-0 items-center gap-1 font-medium text-accent-text hover:text-accent-text-hover">
              {t("usage.view.othersQuotaLink")}
              <Icon name="external" size={12} />
            </button>
          )}
        </span>
      </td>
    </tr>
  );
}
