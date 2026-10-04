import { type EngineIssue, type ProviderId, type ServiceKind, fallbackRows } from "@voltip/shared";
import {
  FallbackSection,
  LampText,
  Segmented,
  Select,
  SettingsPane,
  SettingsRows,
  SettingsSection,
  StatusRow,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useState } from "react";
import { shortModel } from "../../../shell/page-meta";
import { ChineseScript } from "./ChineseScript";
import {
  type SpeechTab,
  SPEECH_TABS,
  languageOptions,
  providersFor,
  serviceTarget,
} from "./helpers";
import { LivePreview } from "./LivePreview";
import { OutputMode, VadTrim } from "./OutputMode";
import { PresetsSection } from "./PresetsSection";
import { ProviderCard } from "./ProviderCard";

/** The 语音模型 group of the settings dialog (docs/dictation.md §3), in two views: 服务商与模型
 *  lists the recognition providers as expandable cards (the one in use is ringed and expanded
 *  first; 使用 switches, the body configures model, endpoint and key, 本机 holds the model library),
 *  识别设置 holds what applies whatever the provider (language, script, live preview, output mode,
 *  silence trimming; how the text is inserted moved to 设置 › 听写 on 2026-09-29). Everything writes
 *  `settings_set_engines` / `provider_key_set`;
 *  everything shown comes from `state.engines`. */
export function SpeechModelsPane() {
  const { t } = useI18n();
  const [tab, setTab] = useState<SpeechTab>("asr");
  return (
    <SettingsPane title={t("engines.title")} lede={t("engines.lede")} data-testid="speech-pane">
      <Segmented<SpeechTab>
        label={t("engines.tabsLabel")}
        value={tab}
        onChange={setTab}
        options={SPEECH_TABS.map((id) => ({
          value: id,
          label: t(`engines.tab.${id}`),
        }))}
        className="self-start"
      />
      {tab === "asr" && <ProviderList kind="asr" />}
      {tab === "options" && <RecognitionOptions />}
    </SettingsPane>
  );
}

/** The AI 模型 group: whether the clean-up runs, and the LLM providers behind it and voice edit, as
 *  the same expandable cards. */
export function AiModelsPane() {
  const { t } = useI18n();
  return (
    <SettingsPane title={t("engines.aiTitle")} lede={t("engines.aiLede")} data-testid="ai-pane">
      <ProviderList kind="llm" />
    </SettingsPane>
  );
}

function ProviderList({ kind }: { kind: ServiceKind }) {
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const engines = state.engines;
  const providers = providersFor(engines, kind);
  const active = kind === "asr" ? engines.asr_provider : engines.llm_provider;
  // The provider in use is open unless the user closed it; any other card is open once the user
  // opened it. Derived during render, so a provider that becomes active opens by itself.
  const [toggled, setToggled] = useState<ReadonlyMap<ProviderId, boolean>>(() => new Map());
  const isOpen = (id: ProviderId) => toggled.get(id) ?? id === active;
  const model = kind === "asr" ? engines.asr_model : engines.refine_model;
  const issue: EngineIssue | undefined = kind === "asr" ? engines.asr_issue : engines.refine_issue;
  // docs/dictation.md §3.5: a fallback model stands in while the selected one is out of quota.
  const { active: inUse } = fallbackRows(engines, kind);
  const standIn = inUse !== undefined && inUse.index !== "selected" ? inUse : undefined;
  const settings = state.settings.engines;
  const toggle = (id: ProviderId, next: boolean) => {
    setToggled((prev) => new Map(prev).set(id, next));
  };
  return (
    <>
      {kind === "llm" && (
        <SettingsRows>
          <StatusRow
            label={t("engines.refineToggle")}
            help={t("engines.refineToggleHelp")}
            note={
              engines.refine_enabled && !engines.refine_ready
                ? t("engines.refineNotReady", {
                    issue: t(`engines.issue.${engines.refine_issue ?? "no_provider"}`),
                  })
                : undefined
            }>
            <Toggle
              checked={settings.refine_enabled}
              onChange={(refine_enabled) => {
                void backend.invoke("settings_set_engines", {
                  engines: { ...settings, refine_enabled },
                });
              }}
              label={settings.refine_enabled ? t("engines.refineOn") : t("engines.refineOff")}
            />
          </StatusRow>
        </SettingsRows>
      )}
      {kind === "llm" && settings.refine_enabled && settings.output_mode === "live_inject" && (
        // docs/dictation.md §12: live_inject pastes each sentence as it closes; nothing is refined.
        <p className="text-[12px] leading-4 text-warning" data-testid="refine-live-inject">
          {t("engines.refineLiveInject")}
        </p>
      )}
      {kind === "llm" && <PresetsSection />}
      <SettingsSection
        title={t(kind === "asr" ? "engines.asrSection.title" : "engines.llmSection.title")}
        description={t(kind === "asr" ? "engines.asrSection.note" : "engines.llmSection.note")}
        data-testid={`providers-${kind}`}
        aside={
          <LampText tone={issue === undefined ? "ok" : "warn"} size="sm">
            <span data-testid={`current-${kind}`}>
              {active === undefined
                ? t("engines.currentNone")
                : standIn !== undefined
                  ? t("engines.currentFallback", {
                      provider: t(`engines.provider.${standIn.provider}`),
                      model: shortModel(standIn.model),
                    })
                  : t("engines.current", {
                      provider: t(`engines.provider.${active}`),
                      model: model.length > 0 ? shortModel(model) : "—",
                    })}
            </span>
          </LampText>
        }>
        {providers.length === 0 ? (
          <p className="text-[12px] text-fg-muted">{t("engines.waiting")}</p>
        ) : (
          <div
            className="flex flex-col gap-3"
            role="list"
            aria-label={t(
              kind === "asr" ? "engines.asrSection.title" : "engines.llmSection.title",
            )}>
            {providers.map((p) => (
              <div role="listitem" key={p.id}>
                <ProviderCard
                  provider={p}
                  kind={kind}
                  open={isOpen(p.id)}
                  onToggle={(next) => {
                    toggle(p.id, next);
                  }}
                />
              </div>
            ))}
          </div>
        )}
        <PrivacyLine kind={kind} />
      </SettingsSection>
      <FallbackSection kind={kind} />
    </>
  );
}

/** Where the service sends its data right now: the provider in use, never the built-in host. */
function PrivacyLine({ kind }: { kind: ServiceKind }) {
  const { t, locale } = useI18n();
  const engines = useUiState().engines;
  const provider = kind === "asr" ? engines.asr_provider : engines.llm_provider;
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
  // docs/dictation.md §3.5: while the chain runs, the providers of the fallback models that can
  // run get the audio (or text) once the models before them run out.
  const others = [
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
  ];
  const text =
    others.length === 0
      ? sent
      : `${sent}${t("engines.privacy.fallbackTargets", {
          targets: others
            .map((p) => t(`engines.provider.${p}`))
            .join(locale === "zh-CN" ? "、" : ", "),
        })}`;
  if (kind === "llm" && provider === undefined) return null;
  return (
    <p className="mono text-[11px] text-fg-subtle" data-testid={`privacy-${kind}`}>
      {text}
    </p>
  );
}

/** Settings that apply whatever the provider. */
function RecognitionOptions() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const settings = state.settings.engines;
  return (
    <>
      <SettingsRows>
        <StatusRow label={t("engines.languageLabel")} help={t("engines.languageHelp")}>
          <Select
            aria-label={t("engines.languageLabel")}
            size="sm"
            value={settings.language ?? ""}
            onChange={(language) => {
              const { language: _old, ...rest } = settings;
              void backend.invoke("settings_set_engines", {
                engines: language.length > 0 ? { ...rest, language } : rest,
              });
            }}
            options={languageOptions(t)}
            // Language names are endonyms (中文 · zh, 日本語 · ja) in every locale.
            data-endonyms=""
          />
        </StatusRow>
      </SettingsRows>
      <ChineseScript />
      <LivePreview />
      <OutputMode />
      <VadTrim />
    </>
  );
}
