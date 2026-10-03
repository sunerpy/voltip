import {
  type ProviderStatus,
  type ServiceKind,
  type ServiceStatus,
  applyProviderDraft,
  checkProviderDraft,
  modelChoices,
  modelDisplayName,
  probeText,
  providerDraft,
  secretStateLabel,
  withProvider,
} from "@voltip/shared";
import { type ReactNode, useState } from "react";
import { useBackend, useUiState } from "../../backend/BackendProvider";
import { Badge } from "../../components/Badge";
import { Button } from "../../components/Button";
import { DisclosureCard } from "../../components/SettingsLayout";
import { type IconName } from "../../components/Icon";
import { IconButton } from "../../components/IconButton";
import { Input } from "../../components/Input";
import { LampText } from "../../components/LampText";
import { Select } from "../../components/Select";
import { useI18n } from "../../i18n/I18nProvider";
import { useFeatureShell } from "../shell";
import { useProviderProbe } from "./useProviderProbe";

/** The model select's last option: type any other model id. */
const OTHER_MODEL = "__other__";

const ICONS: Readonly<Record<ProviderStatus["id"], IconName>> = {
  builtin: "sparkles",
  local: "cpu",
  openai: "cloud",
  groq: "cloud",
  siliconflow: "cloud",
  aliyun: "cloud",
  deepseek: "cloud",
  ollama: "monitor",
  custom: "link",
};

export interface ProviderCardProps {
  provider: ProviderStatus;
  kind: ServiceKind;
  open: boolean;
  onToggle: (open: boolean) => void;
  /** The on-device card's body (the desktop's model library); a shell without local models never
   *  lists that card. */
  localBody?: ReactNode;
}

/** One provider for one service (docs/dictation.md §3): the collapsed header names the provider,
 *  the model in effect and whether it can run, with 使用 beside it; expanded, the built-in card
 *  explains itself, the on-device card holds the shell's `localBody`, and every other card is the
 *  form for its model, endpoint and key plus 测试连接. The desktop's engines pane and the phone's
 *  settings both use it. */
export function ProviderCard({ provider, kind, open, onToggle, localBody }: ProviderCardProps) {
  const { backend } = useBackend();
  const { notify } = useFeatureShell();
  const { t, locale } = useI18n();
  const state = useUiState();
  const service = provider[kind];
  if (service === undefined) return null;
  const name = t(`engines.provider.${provider.id}`);
  const ready = service.issue === undefined;
  const modelName =
    provider.id === "local"
      ? modelDisplayName(
          service.model,
          state.models.find((m) => m.id === service.model)?.name ?? service.model,
          locale,
        )
      : service.model;
  const use = () => {
    void backend.invoke("settings_set_engines", {
      engines: withProvider(state.settings.engines, kind, provider.id),
    });
    notify(t("engines.used", { provider: name }));
  };
  return (
    <DisclosureCard
      open={open}
      onToggle={onToggle}
      icon={ICONS[provider.id]}
      title={name}
      subtitle={modelName.length > 0 ? modelName : t(`engines.providerNote.${provider.id}`)}
      selected={service.active}
      aria-label={name}
      data-testid={`provider-${kind}-${provider.id}`}
      badge={
        <>
          {service.active && <Badge tone="accent">{t("engines.inUse")}</Badge>}
          {provider.on_device && <Badge>{t("engines.onDevice")}</Badge>}
          <Badge tone={ready ? "ok" : "warn"}>
            {ready ? t("engines.ready") : t(`engines.issue.${service.issue ?? "unavailable"}`)}
          </Badge>
        </>
      }
      actions={
        service.active ? undefined : (
          <Button
            size="sm"
            variant="outline"
            onClick={use}
            title={t("engines.useTitle", {
              provider: name,
              service: t(`engines.service.${kind}`),
            })}>
            {t("engines.use")}
          </Button>
        )
      }>
      <p className="text-[12px] leading-4 text-fg-muted">
        {t(`engines.providerNote.${provider.id}`)}
      </p>
      {provider.id === "builtin" ? (
        <BuiltinBody provider={provider} kind={kind} service={service} />
      ) : provider.id === "local" ? (
        localBody
      ) : (
        <ProviderForm
          key={`${provider.id}-${kind}`}
          provider={provider}
          kind={kind}
          service={service}
        />
      )}
    </DisclosureCard>
  );
}

function BuiltinBody({
  provider,
  kind,
  service,
}: {
  provider: ProviderStatus;
  kind: ServiceKind;
  service: ServiceStatus;
}) {
  const { t } = useI18n();
  const probe = useProviderProbe(provider.id, kind);
  return (
    <div className="flex flex-col gap-3">
      <p className="text-[12px] leading-4 text-fg-muted">{t("engines.builtinBody")}</p>
      <div className="mono text-[12px] text-fg">{t("engines.model", { model: service.model })}</div>
      <ProbeRow probe={probe} onRun={() => probe.run({})} />
    </div>
  );
}

function ProbeRow({
  probe,
  onRun,
}: {
  probe: ReturnType<typeof useProviderProbe>;
  onRun: () => void;
}) {
  const { t } = useI18n();
  const result = probe.report ? probeText(probe.report, t) : undefined;
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Button size="sm" variant="outline" icon="refresh" loading={probe.pending} onClick={onRun}>
        {probe.pending ? t("engines.probing") : t("engines.probe")}
      </Button>
      {result && !probe.pending && (
        <span role="status" data-testid="probe-result">
          <LampText tone={result.ok ? "ok" : "danger"} size="sm">
            {result.text}
          </LampText>
        </span>
      )}
    </div>
  );
}

function ProviderForm({
  provider,
  kind,
  service,
}: {
  provider: ProviderStatus;
  kind: ServiceKind;
  service: ServiceStatus;
}) {
  const { backend } = useBackend();
  const { notify } = useFeatureShell();
  const { t, locale } = useI18n();
  const state = useUiState();
  const name = t(`engines.provider.${provider.id}`);
  const [draft, setDraft] = useState(() =>
    providerDraft(state.settings.engines, provider.id, kind),
  );
  const [showKey, setShowKey] = useState(false);
  const [problem, setProblem] = useState<string | undefined>(undefined);
  const probe = useProviderProbe(provider.id, kind);
  const probed = probe.report?.result === "ok" ? (probe.report.models ?? []) : [];
  const choices = modelChoices(service, probed);
  const effectiveModel = draft.model.length > 0 ? draft.model : service.model;
  const [typing, setTyping] = useState(() => choices.length === 0);
  const selectValue = typing || !choices.includes(effectiveModel) ? OTHER_MODEL : effectiveModel;
  const takesKey = provider.key !== "none";
  const shared =
    provider.asr !== undefined && provider.llm !== undefined && provider.id !== "custom";
  const secret = secretStateLabel(service.key, locale);

  const save = () => {
    const issue = checkProviderDraft(draft, provider, kind, t);
    setProblem(issue);
    if (issue !== undefined) return;
    void backend.invoke("settings_set_engines", {
      engines: applyProviderDraft(state.settings.engines, provider.id, kind, draft),
    });
    const key = draft.key.trim();
    if (key.length > 0)
      void backend.invoke("provider_key_set", { provider: provider.id, kind, value: key });
    setDraft({ ...draft, key: "" });
    notify(
      `${t("engines.saved", { provider: name })}${key.length > 0 ? t("engines.savedKey") : ""}`,
    );
  };
  const deleteKey = () => {
    void backend.invoke("provider_key_set", { provider: provider.id, kind, value: null });
    notify(t("engines.keyDeleted", { provider: name }));
  };
  const reset = () => {
    void backend.invoke("settings_set_engines", {
      engines: applyProviderDraft(state.settings.engines, provider.id, kind, {
        model: "",
        baseUrl: "",
      }),
    });
    setDraft({ model: "", baseUrl: "", key: "" });
    setTyping(false);
    setProblem(undefined);
    notify(t("engines.resetDone", { provider: name }));
  };

  return (
    <div className="flex flex-col gap-3" data-testid="provider-form">
      <div className="grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(240px,1fr))]">
        {choices.length > 0 && (
          <Select
            label={t("engines.field.model")}
            size="sm"
            mono
            value={selectValue}
            onChange={(value) => {
              if (value === OTHER_MODEL) {
                // A fresh field: the current model stays the placeholder.
                setTyping(true);
                setDraft({ ...draft, model: "" });
                return;
              }
              setTyping(false);
              setDraft({ ...draft, model: value });
            }}
            options={[
              ...choices.map((m) => ({ value: m, label: m })),
              { value: OTHER_MODEL, label: t("engines.field.modelOther") },
            ]}
          />
        )}
        {selectValue === OTHER_MODEL && (
          <Input
            label={t("engines.field.modelCustom")}
            mono
            size="sm"
            value={draft.model}
            placeholder={service.model}
            onChange={(e) => {
              setDraft({ ...draft, model: e.target.value });
            }}
          />
        )}
        <Input
          label={t("engines.field.baseUrl")}
          mono
          size="sm"
          value={draft.baseUrl}
          placeholder={service.default_base_url ?? "http://127.0.0.1:8000/v1"}
          onChange={(e) => {
            setDraft({ ...draft, baseUrl: e.target.value });
          }}
          help={
            service.default_base_url === undefined
              ? t("engines.field.baseUrlHelpCustom")
              : provider.id === "aliyun"
                ? t("engines.field.baseUrlHelpAliyun")
                : t("engines.field.baseUrlHelpVendor")
          }
        />
      </div>
      <p className="text-[12px] leading-4 text-fg-muted">{t("engines.field.modelHelp")}</p>
      {takesKey && (
        <div className="flex items-end gap-1">
          <Input
            label={
              provider.key === "required" ? t("engines.field.key") : t("engines.field.keyOptional")
            }
            mono
            size="sm"
            type={showKey ? "text" : "password"}
            autoComplete="off"
            value={draft.key}
            placeholder={
              service.key.set
                ? t("engines.field.keyPlaceholderSet")
                : t("engines.field.keyPlaceholderUnset")
            }
            onChange={(e) => {
              setDraft({ ...draft, key: e.target.value });
            }}
            className="flex-1"
            help={
              shared ? t("engines.field.keyShared", { provider: name }) : t("engines.field.keyHelp")
            }
          />
          <IconButton
            icon="eye"
            label={showKey ? t("engines.field.hideKey") : t("engines.field.showKey")}
            size={28}
            bordered
            onClick={() => {
              setShowKey(!showKey);
            }}
            className="mb-5"
          />
        </div>
      )}
      {takesKey && (
        <div className="flex items-center gap-2 text-[12px]">
          <span className="text-fg-subtle">{t("engines.field.keyState")}</span>
          <LampText tone={secret.tone} size="sm">
            <span data-testid="provider-key-state">{secret.text}</span>
          </LampText>
          {provider.console && (
            <Button
              size="sm"
              variant="text"
              icon="external"
              onClick={() => {
                void backend.providerConsoleOpen(provider.id);
              }}>
              {t("engines.getKey")}
            </Button>
          )}
        </div>
      )}
      <ProbeRow
        probe={probe}
        onRun={() => {
          probe.run({ baseUrl: draft.baseUrl, key: draft.key });
        }}
      />
      {problem !== undefined && (
        <p className="text-[12px] text-danger" role="alert" data-testid="provider-problem">
          {problem}
        </p>
      )}
      <div className="flex flex-wrap items-center justify-end gap-2 border-t border-border pt-3">
        {takesKey && service.key.source === "user" && (
          <Button size="sm" variant="text-danger" icon="trash" onClick={deleteKey}>
            {t("engines.deleteKey")}
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={reset}>
          {t("engines.reset")}
        </Button>
        <Button size="sm" variant="primary" onClick={save}>
          {t("engines.save")}
        </Button>
      </div>
    </div>
  );
}
