import {
  type ServiceKind,
  fallbackRows,
  fallbackStatusOf,
  serviceTarget,
  shortModel,
} from "@voltip/shared";
import { useUiState } from "../../backend/BackendProvider";
import { LampText } from "../../components/LampText";
import { useI18n } from "../../i18n/I18nProvider";

/** 当前：… for `kind`: the provider and model requests go to now — the fallback model, marked as
 *  such, while it stands in for the selected one (docs/dictation.md §3.5). The desktop's provider
 *  sections show it beside their title, the phone's under their cards. */
export function CurrentService({ kind }: { kind: ServiceKind }) {
  const { t } = useI18n();
  const engines = useUiState().engines;
  const provider = kind === "asr" ? engines.asr_provider : engines.llm_provider;
  const model = kind === "asr" ? engines.asr_model : engines.refine_model;
  const issue = kind === "asr" ? engines.asr_issue : engines.refine_issue;
  const { active } = fallbackRows(engines, kind);
  const standIn = active !== undefined && active.index !== "selected" ? active : undefined;
  return (
    <LampText tone={issue === undefined ? "ok" : "warn"} size="sm">
      <span data-testid={`current-${kind}`}>
        {provider === undefined
          ? t("engines.currentNone")
          : standIn !== undefined
            ? t("engines.currentFallback", {
                provider: t(`engines.provider.${standIn.provider}`),
                model: shortModel(standIn.model),
              })
            : t("engines.current", {
                provider: t(`engines.provider.${provider}`),
                model: model.length > 0 ? shortModel(model) : "—",
              })}
      </span>
    </LampText>
  );
}

/** Where `kind` sends its data now: the provider in use (never the built-in service's host), and
 *  while the fallback models run, the providers of those that can (docs/dictation.md §3.5) —
 *  with the switch off, or a selected service the chain does not run for, nobody else. Nothing
 *  for a clean-up without a provider. */
export function ServicePrivacy({ kind }: { kind: ServiceKind }) {
  const { t, locale } = useI18n();
  const engines = useUiState().engines;
  const provider = kind === "asr" ? engines.asr_provider : engines.llm_provider;
  if (kind === "llm" && provider === undefined) return null;
  const host = kind === "asr" ? engines.asr_host : engines.refine_host;
  const target = serviceTarget(provider, host, t);
  const sent =
    kind === "asr"
      ? target === undefined
        ? t("engines.privacy.audioLocal")
        : t("engines.privacy.audioSent", { target })
      : target === undefined
        ? t("engines.privacy.textLocal")
        : t("engines.privacy.textSent", { target });
  const others = fallbackStatusOf(engines, kind).in_use
    ? [
        ...new Set(
          fallbackRows(engines, kind)
            .rows.filter(
              (r) =>
                r.index !== "selected" &&
                (r.state === "ready" || r.state === "active" || r.state === "exhausted"),
            )
            .map((r) => r.provider)
            .filter((p) => p !== provider),
        ),
      ]
    : [];
  const text =
    others.length === 0
      ? sent
      : `${sent}${t("engines.privacy.fallbackTargets", {
          targets: others
            .map((p) => t(`engines.provider.${p}`))
            .join(locale === "zh-CN" ? "、" : ", "),
        })}`;
  return (
    <p className="mono text-[11px] text-fg-subtle" data-testid={`privacy-${kind}`}>
      {text}
    </p>
  );
}
