// The recognition and clean-up providers (docs/dictation.md §3), `@voltip/ui`'s ProviderCard,
// CurrentService, ServicePrivacy and ChineseScript on native views: one card per provider that names
// it, the model in effect and whether it can run, with 使用 beside it; opened, the built-in card
// explains itself and every other card is the form for its model, endpoint and key plus 测试连接.
// The drafts, checks and labels are `@voltip/shared`'s, as on the desktop.
import {
  CHINESE_SCRIPTS,
  type ProbeReport,
  type ProviderId,
  type ProviderStatus,
  REASONING_EFFORTS,
  type ServiceKind,
  type ServiceStatus,
  applyProviderDraft,
  checkProviderDraft,
  choosesInterface,
  fallbackRows,
  fallbackStatusOf,
  modelChoices,
  probeText,
  providerDraft,
  secretStateLabel,
  serviceTarget,
  shortModel,
  withProvider,
} from "@voltip/shared";
import { useCallback, useEffect, useRef, useState } from "react";
import { View } from "react-native";
import {
  Icon,
  IconButton,
  SegmentedButtons,
  Text,
  TextInput,
  TouchableRipple,
} from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { CARD_RADIUS, Hint, Mono, StateLine, type Tone, useAppTheme } from "../ui/kit";
import { SelectField } from "../ui/Select";

/** The model select's last option: type any other model id. */
export const OTHER_MODEL = "__other__";

const ICONS: Readonly<Record<ProviderId, string>> = {
  builtin: "creation-outline",
  local: "chip",
  openai: "cloud-outline",
  groq: "cloud-outline",
  google: "cloud-outline",
  siliconflow: "cloud-outline",
  aliyun: "cloud-outline",
  deepseek: "cloud-outline",
  ollama: "monitor",
  custom: "link-variant",
};

/** How long a card waits for a `provider_probe` answer before it says the test timed out. */
export const PROBE_WAIT_MS = 20_000;

/** 测试连接 for one provider card (`@voltip/ui`'s useProviderProbe): sends `provider_probe` and
 *  listens for the `provider_probe` event that answers it. */
export function useProviderProbe(provider: ProviderId, kind: ServiceKind) {
  const { backend } = useBackend();
  const [pending, setPending] = useState(false);
  const [report, setReport] = useState<ProbeReport | undefined>(undefined);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(
    () =>
      backend.on((event) => {
        if (event.type !== "provider_probe" || event.provider !== provider || event.kind !== kind)
          return;
        const { type: _type, ...answer } = event;
        clearTimeout(timer.current);
        setPending(false);
        setReport(answer);
      }),
    [backend, provider, kind],
  );
  useEffect(
    () => () => {
      clearTimeout(timer.current);
    },
    [],
  );
  const run = useCallback(
    (draft: { baseUrl?: string; key?: string }) => {
      const baseUrl = draft.baseUrl?.trim();
      const key = draft.key?.trim();
      setPending(true);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        setPending(false);
        setReport({ provider, kind, result: "failed", reason: "timeout" });
      }, PROBE_WAIT_MS);
      void backend
        .invoke("provider_probe", {
          provider,
          kind,
          baseUrl: baseUrl === undefined || baseUrl.length === 0 ? null : baseUrl,
          key: key === undefined || key.length === 0 ? null : key,
        })
        .catch(() => {
          clearTimeout(timer.current);
          setPending(false);
          setReport({ provider, kind, result: "failed", reason: "unsupported" });
        });
    },
    [backend, provider, kind],
  );
  return { pending, report, run };
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
    <View style={{ gap: 8 }}>
      <Button
        mode="outlined"
        icon="refresh"
        loading={probe.pending}
        style={{ alignSelf: "flex-start" }}
        onPress={onRun}>
        {probe.pending ? t("engines.probing") : t("engines.probe")}
      </Button>
      {result !== undefined && !probe.pending && (
        <View testID="probe-result" accessibilityLiveRegion="polite">
          <StateLine tone={result.ok ? "ok" : "danger"} small>
            {result.text}
          </StateLine>
        </View>
      )}
    </View>
  );
}

/** A small status label in a soft shade (使用中, 就绪, a provider's problem). */
function Tag({
  tone,
  children,
}: {
  tone: "accent" | "ok" | "warning" | "neutral";
  children: string;
}) {
  const theme = useAppTheme();
  const shades: Record<"accent" | "ok" | "warning" | "neutral", readonly [string, string]> = {
    accent: [theme.colors.primaryContainer, theme.colors.onPrimaryContainer],
    ok: [theme.voltip.okSoft, theme.voltip.okText],
    warning: [theme.voltip.warningSoft, theme.colors.onSurface],
    neutral: [theme.colors.surfaceVariant, theme.colors.onSurfaceVariant],
  };
  const [bg, fg] = shades[tone];
  return (
    <View
      style={{ backgroundColor: bg, borderRadius: 8, paddingHorizontal: 8, paddingVertical: 2 }}>
      <Text variant="labelSmall" style={{ color: fg }}>
        {children}
      </Text>
    </View>
  );
}

export function ProviderCard({
  provider,
  kind,
  open,
  onToggle,
}: {
  provider: ProviderStatus;
  kind: ServiceKind;
  open: boolean;
  onToggle: (open: boolean) => void;
}) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const state = useUiState();
  const service = provider[kind];
  if (service === undefined) return null;
  const name = t(`engines.provider.${provider.id}`);
  const ready = service.issue === undefined;
  const use = () => {
    void backend.invoke("settings_set_engines", {
      engines: withProvider(state.settings.engines, kind, provider.id),
    });
    shell.toast(t("engines.used", { provider: name }));
  };
  return (
    <View
      testID={`provider-${kind}-${provider.id}`}
      style={{
        borderRadius: CARD_RADIUS,
        borderWidth: service.active ? 2 : 1,
        borderColor: service.active ? theme.colors.primary : theme.colors.outlineVariant,
        backgroundColor: theme.colors.surface,
        overflow: "hidden",
      }}>
      <TouchableRipple
        onPress={() => {
          onToggle(!open);
        }}
        accessibilityRole="button"
        accessibilityLabel={name}
        accessibilityState={{ expanded: open, selected: service.active }}>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 12, padding: 16 }}>
          <Icon
            source={ICONS[provider.id]}
            size={24}
            color={service.active ? theme.colors.primary : theme.colors.onSurfaceVariant}
          />
          <View style={{ flex: 1, gap: 4 }}>
            <Text variant="titleMedium">{name}</Text>
            <Mono numberOfLines={1}>
              {service.model.length > 0 ? service.model : t(`engines.providerNote.${provider.id}`)}
            </Mono>
            <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 6 }}>
              {service.active && <Tag tone="accent">{t("engines.inUse")}</Tag>}
              <Tag tone={ready ? "ok" : "warning"}>
                {ready ? t("engines.ready") : t(`engines.issue.${service.issue ?? "unavailable"}`)}
              </Tag>
            </View>
          </View>
          {!service.active && (
            <Button
              mode="outlined"
              compact
              onPress={use}
              accessibilityLabel={t("engines.useTitle", {
                provider: name,
                service: t(`engines.service.${kind}`),
              })}>
              {t("engines.use")}
            </Button>
          )}
          <Icon
            source={open ? "chevron-up" : "chevron-down"}
            size={24}
            color={theme.colors.onSurfaceVariant}
          />
        </View>
      </TouchableRipple>
      {open && (
        <View style={{ paddingHorizontal: 16, paddingBottom: 16, gap: 12 }}>
          <Hint>{t(`engines.providerNote.${provider.id}`)}</Hint>
          {provider.id === "builtin" ? (
            <BuiltinBody provider={provider} kind={kind} service={service} />
          ) : (
            <ProviderForm
              key={`${provider.id}-${kind}`}
              provider={provider}
              kind={kind}
              service={service}
            />
          )}
        </View>
      )}
    </View>
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
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const state = useUiState();
  const probe = useProviderProbe(provider.id, kind);
  // The build may offer several models (user request 2026-10-08); the first is its default, kept
  // as no choice at all so that a newer build's default follows.
  const choose = (model: string) => {
    void backend.invoke("settings_set_engines", {
      engines: applyProviderDraft(state.settings.engines, provider.id, kind, {
        model: model === service.presets[0] ? "" : model,
        baseUrl: "",
      }),
    });
    shell.toast(t("engines.saved", { provider: t(`engines.provider.${provider.id}`) }));
  };
  return (
    <View style={{ gap: 12 }}>
      <Hint>{t("engines.builtinBody")}</Hint>
      {service.presets.length > 1 ? (
        <SelectField
          label={t("engines.field.model")}
          value={service.model}
          onChange={choose}
          options={service.presets.map((m) => ({ value: m, label: m }))}
          testID={`builtin-${kind}-model`}
        />
      ) : (
        <Mono>{t("engines.model", { model: service.model })}</Mono>
      )}
      <ProbeRow
        probe={probe}
        onRun={() => {
          probe.run({});
        }}
      />
    </View>
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
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
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
    shell.toast(
      `${t("engines.saved", { provider: name })}${key.length > 0 ? t("engines.savedKey") : ""}`,
    );
  };
  const deleteKey = () => {
    void backend.invoke("provider_key_set", { provider: provider.id, kind, value: null });
    shell.toast(t("engines.keyDeleted", { provider: name }));
  };
  const reset = () => {
    void backend.invoke("settings_set_engines", {
      engines: applyProviderDraft(state.settings.engines, provider.id, kind, {
        model: "",
        baseUrl: "",
      }),
    });
    setDraft(
      choosesInterface(provider.id, kind)
        ? { model: "", baseUrl: "", key: "", api: "chat_completions" }
        : { model: "", baseUrl: "", key: "" },
    );
    setTyping(false);
    setProblem(undefined);
    shell.toast(t("engines.resetDone", { provider: name }));
  };
  const keyTone: Tone = secret.tone === "ok" ? "ok" : secret.tone === "warn" ? "warning" : "idle";

  return (
    <View style={{ gap: 12 }} testID="provider-form">
      {choices.length > 0 && (
        <SelectField
          label={t("engines.field.model")}
          value={selectValue}
          onChange={(value) => {
            if (value === OTHER_MODEL) {
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
        <TextInput
          mode="outlined"
          label={t("engines.field.modelCustom")}
          value={draft.model}
          placeholder={service.model}
          autoCapitalize="none"
          autoCorrect={false}
          onChangeText={(model) => {
            setDraft({ ...draft, model });
          }}
        />
      )}
      <View style={{ gap: 4 }}>
        <TextInput
          mode="outlined"
          label={t("engines.field.baseUrl")}
          value={draft.baseUrl}
          placeholder={service.default_base_url ?? "http://127.0.0.1:8000/v1"}
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType="url"
          onChangeText={(baseUrl) => {
            setDraft({ ...draft, baseUrl });
          }}
        />
        <Hint>
          {service.default_base_url === undefined
            ? t("engines.field.baseUrlHelpCustom")
            : provider.id === "aliyun"
              ? t("engines.field.baseUrlHelpAliyun")
              : t("engines.field.baseUrlHelpVendor")}
        </Hint>
      </View>
      {choosesInterface(provider.id, kind) && (
        // docs/dictation.md §3.7: the custom clean-up's interface, and the Responses effort.
        <View style={{ gap: 8 }}>
          <SegmentedButtons
            value={draft.api ?? "chat_completions"}
            onValueChange={(value) => {
              setDraft({ ...draft, api: value === "responses" ? "responses" : "chat_completions" });
            }}
            buttons={[
              { value: "chat_completions", label: "Chat Completions" },
              { value: "responses", label: "Responses" },
            ]}
          />
          {draft.api === "responses" && (
            <SelectField
              label={t("engines.field.reasoning")}
              value={draft.reasoning ?? ""}
              onChange={(value) => {
                const reasoning = REASONING_EFFORTS.find((e) => e === value);
                const next = { ...draft };
                if (reasoning === undefined) delete next.reasoning;
                else next.reasoning = reasoning;
                setDraft(next);
              }}
              options={[
                { value: "", label: t("engines.field.reasoningNone") },
                ...REASONING_EFFORTS.map((e) => ({ value: e, label: e })),
              ]}
            />
          )}
          <Hint>
            {t("engines.field.apiHelp")}
            {draft.api === "responses" ? ` ${t("engines.field.reasoningHelp")}` : ""}
          </Hint>
        </View>
      )}
      <Hint>{t("engines.field.modelHelp")}</Hint>
      {takesKey && (
        <View style={{ gap: 4 }}>
          <TextInput
            mode="outlined"
            label={
              provider.key === "required" ? t("engines.field.key") : t("engines.field.keyOptional")
            }
            value={draft.key}
            placeholder={
              service.key.set
                ? t("engines.field.keyPlaceholderSet")
                : t("engines.field.keyPlaceholderUnset")
            }
            secureTextEntry={!showKey}
            autoCapitalize="none"
            autoCorrect={false}
            autoComplete="off"
            onChangeText={(key) => {
              setDraft({ ...draft, key });
            }}
            right={
              <TextInput.Icon
                icon={showKey ? "eye-off-outline" : "eye-outline"}
                accessibilityLabel={
                  showKey ? t("engines.field.hideKey") : t("engines.field.showKey")
                }
                onPress={() => {
                  setShowKey(!showKey);
                }}
              />
            }
          />
          <Hint>
            {shared ? t("engines.field.keyShared", { provider: name }) : t("engines.field.keyHelp")}
          </Hint>
          <View style={{ flexDirection: "row", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
            <Text variant="bodySmall" style={{ color: theme.voltip.subtle }}>
              {t("engines.field.keyState")}
            </Text>
            <View testID="provider-key-state">
              <StateLine tone={keyTone} small>
                {secret.text}
              </StateLine>
            </View>
            {provider.console && (
              <Button
                compact
                icon="open-in-new"
                onPress={() => void backend.providerConsoleOpen(provider.id)}>
                {t("engines.getKey")}
              </Button>
            )}
          </View>
        </View>
      )}
      <ProbeRow
        probe={probe}
        onRun={() => {
          probe.run({ baseUrl: draft.baseUrl, key: draft.key });
        }}
      />
      {problem !== undefined && (
        <Text
          variant="bodySmall"
          accessibilityRole="alert"
          testID="provider-problem"
          style={{ color: theme.colors.error }}>
          {problem}
        </Text>
      )}
      <View
        style={{
          flexDirection: "row",
          flexWrap: "wrap",
          justifyContent: "flex-end",
          alignItems: "center",
          gap: 8,
        }}>
        {takesKey && service.key.source === "user" && (
          <IconButton
            icon="trash-can-outline"
            iconColor={theme.colors.error}
            accessibilityLabel={t("engines.deleteKey")}
            onPress={deleteKey}
          />
        )}
        <Button onPress={reset}>{t("engines.reset")}</Button>
        <Button mode="contained" onPress={save} testID="provider-save">
          {t("engines.save")}
        </Button>
      </View>
    </View>
  );
}

/** Which provider cards are open: the one in use unless the user closed it, any other once the
 *  user opened it. */
export function useOpenCards(active: ProviderId | undefined) {
  const [toggled, setToggled] = useState<ReadonlyMap<ProviderId, boolean>>(() => new Map());
  return {
    isOpen: (id: ProviderId) => toggled.get(id) ?? id === active,
    toggle: (id: ProviderId, open: boolean) => {
      setToggled((prev) => new Map(prev).set(id, open));
    },
  };
}

/** 当前：… for `kind`: the provider and model requests go to now — the fallback model, marked as
 *  such, while it stands in for the selected one (docs/dictation.md §3.5). */
export function CurrentService({ kind }: { kind: ServiceKind }) {
  const { t } = useI18n();
  const engines = useUiState().engines;
  const provider = kind === "asr" ? engines.asr_provider : engines.llm_provider;
  const model = kind === "asr" ? engines.asr_model : engines.refine_model;
  const issue = kind === "asr" ? engines.asr_issue : engines.refine_issue;
  const { active } = fallbackRows(engines, kind);
  const standIn = active !== undefined && active.index !== "selected" ? active : undefined;
  return (
    <View testID={`current-${kind}`}>
      <StateLine tone={issue === undefined ? "ok" : "warning"} small>
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
      </StateLine>
    </View>
  );
}

/** Where `kind` sends its data now: the provider in use, and while the fallback models run, the
 *  providers of those that can (docs/dictation.md §3.5). */
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
      : `${sent}${t("engines.privacy.fallbackTargets", { targets: others.map((p) => t(`engines.provider.${p}`)).join(locale === "zh-CN" ? "、" : ", ") })}`;
  return <Mono>{text}</Mono>;
}

const SCRIPT_KEYS = {
  simplified: "engines.chineseScript.simplified",
  traditional: "engines.chineseScript.traditional",
  as_is: "engines.chineseScript.asIs",
} as const;

/** 中文字形 (docs/dictation.md §17): the script the core brings every recogniser's Chinese to. */
export function ChineseScript() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const settings = useUiState().settings.engines;
  return (
    <View style={{ gap: 8 }} testID="chinese-script">
      <Text variant="titleSmall">{t("engines.chineseScript.title")}</Text>
      <Hint>{t("engines.chineseScript.note")}</Hint>
      <SegmentedButtons
        value={settings.chinese_script}
        onValueChange={(value) => {
          const chinese_script = value;
          if (chinese_script !== settings.chinese_script)
            void backend.invoke("settings_set_engines", {
              engines: { ...settings, chinese_script },
            });
        }}
        buttons={CHINESE_SCRIPTS.map((script) => ({
          value: script,
          label: t(SCRIPT_KEYS[script]),
        }))}
      />
    </View>
  );
}
