import {
  type EngineSettings,
  type ModelState,
  type ServiceKind,
  type UsageNotice,
  resetCountdown,
  resetLabel,
  serviceUsage,
  usableFallback,
  usageAmount,
  usageNotice,
  usesBuiltin,
  withProvider,
} from "@voltip/shared";
import { Banner, Button, useBackend, useI18n, useUiState, useUsageNow } from "@voltip/ui";

/** The on-device model 改用本机模型 switches to: the one the settings select when it is installed,
 *  else the recommended installed one, else any installed one; none when nothing is installed. */
export function installedLocalModel(
  models: readonly ModelState[],
  settings: EngineSettings,
): ModelState | undefined {
  const installed = models.filter((m) => m.state.kind === "installed");
  return (
    installed.find((m) => m.id === settings.local_model) ??
    installed.find((m) => m.recommended) ??
    installed[0]
  );
}

/** Switch recognition to the on-device `model`. */
export function useSwitchToLocal(): (model: ModelState) => void {
  const { backend } = useBackend();
  const state = useUiState();
  return (model) => {
    void backend.invoke("settings_set_engines", {
      engines: { ...withProvider(state.settings.engines, "asr", "local"), local_model: model.id },
    });
  };
}

export interface UsageNoticesProps {
  openSpeech: () => void;
  /** The speech page with the on-device models' drawer open. */
  openLocalModels: () => void;
  openAi: () => void;
}

/** The home page's usage notices (Penpot 「用量与额度」 boards 11 and 13): the built-in recognition or
 *  clean-up near its day's limit (from 80 %) or at it, while it is the service in use and limits
 *  apply. Each says when the day starts again and what can be done meanwhile; closed, it stays away
 *  for the rest of the day (`usage_notice_dismiss`). */
export function UsageNotices(props: UsageNoticesProps) {
  return (
    <>
      <ServiceNotice kind="asr" {...props} />
      <ServiceNotice kind="llm" {...props} />
    </>
  );
}

function ServiceNotice({
  kind,
  openSpeech,
  openLocalModels,
  openAi,
}: UsageNoticesProps & { kind: ServiceKind }) {
  const { t, locale } = useI18n();
  const { backend } = useBackend();
  const state = useUiState();
  const now = useUsageNow();
  const switchToLocal = useSwitchToLocal();
  const usage = serviceUsage(state.usage, kind, now);
  const notice: UsageNotice | undefined = usageNotice(
    usage,
    state.usage,
    usesBuiltin(state.engines, kind),
  );
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
  const local =
    kind === "asr" ? installedLocalModel(state.models, state.settings.engines) : undefined;
  const out = usage.tone === "out";
  const body =
    (notice === "asr_out" || notice === "refine_out") && fallback !== undefined
      ? t(`usage.notice.${notice}.fallback`, { reset, countdown, model: fallback.model })
      : t(`usage.notice.${notice}.body`, { reset, countdown });
  const open =
    kind === "asr" ? (
      <Button size="sm" variant="outline" onClick={openSpeech}>
        {t("usage.notice.openSpeech")}
      </Button>
    ) : (
      <Button size="sm" variant="outline" onClick={openAi}>
        {t("usage.notice.openAi")}
      </Button>
    );
  return (
    <Banner
      tone={out ? "danger" : "warn"}
      marker="bar"
      title={t(`usage.notice.${notice}.title`, { amount })}
      onDismiss={() => {
        void backend.invoke("usage_notice_dismiss", { notice });
      }}
      actions={
        <>
          {kind === "asr" && out && fallback === undefined && (
            <Button
              size="sm"
              variant="primary"
              data-testid="usage-use-local"
              onClick={() => {
                if (local === undefined) openLocalModels();
                else switchToLocal(local);
              }}>
              {local === undefined ? t("usage.notice.downloadLocal") : t("usage.notice.useLocal")}
            </Button>
          )}
          {open}
        </>
      }>
      <p data-testid={`usage-notice-${notice}`}>{body}</p>
    </Banner>
  );
}
