// The recognition and clean-up providers (docs/dictation.md §3), `@voltip/ui`'s ProviderCards,
// CurrentService, ServicePrivacy and ChineseScript on native views: the providers as cards two to a
// row that name each one, whether it can run and the model in effect; a tap opens a sheet from the
// bottom where the built-in service explains itself and every other provider is the form for its
// model, endpoint and key plus 测试连接, with 保存 and 设为当前. The drafts, checks and labels are
// `@voltip/shared`'s, as on the desktop.
import {
  CHINESE_SCRIPTS,
  type ProbeReport,
  type ProviderBadgeTone,
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
  providerBadges,
  providerDraft,
  secretStateLabel,
  serviceTarget,
  shortModel,
  withProvider,
} from "@voltip/shared";
import { useCallback, useEffect, useRef, useState } from "react";
import { ScrollView, View } from "react-native";
import { Icon, SegmentedButtons, Text, TextInput, TouchableRipple } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { CARD_RADIUS, Hint, Lamp, Mono, StateLine, type Tone, useAppTheme } from "../ui/kit";
import { SelectField } from "../ui/Select";
import { BottomSheet } from "../ui/Sheet";
import { mix } from "../ui/statTile";

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

/** 测试连接 and what it answered, or that it has not run yet, on a block of its own. */
function ProbePanel({
  probe,
  onRun,
}: {
  probe: ReturnType<typeof useProviderProbe>;
  onRun: () => void;
}) {
  const theme = useAppTheme();
  const { t } = useI18n();
  const result = probe.report ? probeText(probe.report, t) : undefined;
  return (
    <View
      style={{
        flexDirection: "row",
        flexWrap: "wrap",
        alignItems: "center",
        gap: 12,
        borderRadius: 12,
        paddingHorizontal: 8,
        paddingVertical: 6,
        backgroundColor: theme.voltip.inset,
      }}>
      <Button icon="refresh" loading={probe.pending} onPress={onRun} testID="provider-probe">
        {probe.pending ? t("engines.probing") : t("engines.probe")}
      </Button>
      {result !== undefined && !probe.pending ? (
        <View testID="probe-result" accessibilityLiveRegion="polite" style={{ flexShrink: 1 }}>
          <StateLine tone={result.ok ? "ok" : "danger"} small>
            {result.text}
          </StateLine>
        </View>
      ) : (
        !probe.pending && (
          <Text variant="bodySmall" style={{ color: theme.voltip.subtle }}>
            {t("engines.probeIdle")}
          </Text>
        )
      )}
    </View>
  );
}

/** A small status label in a soft shade (使用中, 就绪, a provider's problem). */
function Tag({ tone, children }: { tone: ProviderBadgeTone; children: string }) {
  const theme = useAppTheme();
  const shades: Record<ProviderBadgeTone, readonly [string, string]> = {
    accent: [theme.colors.primaryContainer, theme.colors.onPrimaryContainer],
    ok: [theme.voltip.okSoft, theme.voltip.okText],
    warn: [theme.voltip.warningSoft, theme.colors.onSurface],
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

/** The provider's icon on a block: a soft accent one for the provider in use, never a solid one
 *  (user feedback 2026-10-10 on the drafts). */
function ProviderIcon({
  id,
  active,
  size = 40,
}: {
  id: ProviderId;
  active: boolean;
  size?: number;
}) {
  const theme = useAppTheme();
  return (
    <View
      style={{
        width: size,
        height: size,
        borderRadius: 12,
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: active ? theme.colors.primaryContainer : theme.colors.surfaceVariant,
      }}>
      <Icon
        source={ICONS[id]}
        size={Math.round(size * 0.55)}
        color={active ? theme.colors.onPrimaryContainer : theme.colors.onSurfaceVariant}
      />
    </View>
  );
}

/** Switches `kind` to `provider`. */
function useSwitch(provider: ProviderStatus, kind: ServiceKind) {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const state = useUiState();
  return () => {
    void backend.invoke("settings_set_engines", {
      engines: withProvider(state.settings.engines, kind, provider.id),
    });
    shell.toast(t("engines.used", { provider: t(`engines.provider.${provider.id}`) }));
  };
}

/** The providers of one service (docs/dictation.md §3) as cards two to a row (user decision
 *  2026-10-10, Penpot 「Voltip 服务商设置」): each names the provider, whether it can run and the
 *  model in effect; a tap opens its settings in a sheet from the bottom. The on-device provider is
 *  never listed: a phone has no local models. */
export function ProviderCards({
  kind,
  providers,
  label,
}: {
  kind: ServiceKind;
  providers: readonly ProviderStatus[];
  label: string;
}) {
  const [openId, setOpenId] = useState<ProviderId | undefined>(undefined);
  // The provider the sheet shows: it stays while the sheet slides away, and each opening is a
  // fresh form.
  const [shown, setShown] = useState<{ id: ProviderId; opening: number } | undefined>(undefined);
  const rows: ProviderStatus[][] = [];
  for (let at = 0; at < providers.length; at += 2) rows.push(providers.slice(at, at + 2));
  const provider = providers.find((p) => p.id === shown?.id);
  return (
    <View style={{ gap: 12 }} accessibilityLabel={label}>
      {rows.map((row) => (
        <View key={row.map((p) => p.id).join("+")} style={{ flexDirection: "row", gap: 12 }}>
          {row.map((p) => (
            <ProviderTile
              key={p.id}
              provider={p}
              kind={kind}
              open={p.id === openId}
              onOpen={() => {
                setOpenId(p.id);
                setShown((prev) => ({ id: p.id, opening: (prev?.opening ?? 0) + 1 }));
              }}
            />
          ))}
          {row.length === 1 && <View style={{ flex: 1 }} />}
        </View>
      ))}
      {provider !== undefined && shown !== undefined && (
        <ProviderSheet
          key={`${provider.id}-${kind}-${shown.opening}`}
          provider={provider}
          kind={kind}
          open={openId !== undefined}
          onClose={() => {
            setOpenId(undefined);
          }}
        />
      )}
    </View>
  );
}

function ProviderTile({
  provider,
  kind,
  open,
  onOpen,
}: {
  provider: ProviderStatus;
  kind: ServiceKind;
  open: boolean;
  onOpen: () => void;
}) {
  const theme = useAppTheme();
  const { t } = useI18n();
  const service = provider[kind];
  if (service === undefined) return null;
  const name = t(`engines.provider.${provider.id}`);
  const badges = providerBadges(service, t);
  // The dot and the line take the most pressing state: a problem before 使用中.
  const tone: ProviderBadgeTone = badges.some((b) => b.tone === "warn")
    ? "warn"
    : (badges[0]?.tone ?? "ok");
  const lamp: Tone = tone === "warn" ? "warning" : tone;
  const ink = { accent: theme.colors.primary, ok: theme.voltip.okText, warn: theme.voltip.warning };
  const status = badges.map((b) => b.text).join(" · ");
  return (
    <View
      testID={`provider-${kind}-${provider.id}`}
      style={{
        flex: 1,
        borderRadius: CARD_RADIUS,
        borderWidth: open ? 2 : 1,
        borderColor: open
          ? theme.colors.primary
          : service.active
            ? mix(theme.colors.outlineVariant, theme.colors.primary, 0.45)
            : theme.colors.outlineVariant,
        backgroundColor: service.active
          ? mix(theme.colors.surface, theme.colors.primary, theme.dark ? 0.1 : 0.05)
          : theme.colors.surface,
        overflow: "hidden",
      }}>
      <TouchableRipple
        onPress={onOpen}
        accessibilityRole="button"
        accessibilityLabel={name}
        accessibilityHint={status}
        accessibilityState={{ expanded: open, selected: service.active }}
        style={{ flex: 1, padding: 14 }}>
        <View style={{ gap: 10 }}>
          <View
            style={{
              flexDirection: "row",
              justifyContent: "space-between",
              alignItems: "flex-start",
            }}>
            <ProviderIcon id={provider.id} active={service.active} />
            <Lamp tone={lamp} testID={`provider-${kind}-${provider.id}-lamp`} />
          </View>
          <View style={{ gap: 2 }}>
            <Text variant="titleSmall" numberOfLines={1} style={{ fontWeight: "700" }}>
              {name}
            </Text>
            <Text variant="labelMedium" numberOfLines={2} style={{ color: ink[tone] }}>
              {status}
            </Text>
          </View>
          <Mono numberOfLines={2} testID={`provider-${kind}-${provider.id}-model`}>
            {service.model.length > 0 ? service.model : "—"}
          </Mono>
        </View>
      </TouchableRipple>
    </View>
  );
}

/** One provider's settings for one service, in the sheet a card opens. */
function ProviderSheet({
  provider,
  kind,
  open,
  onClose,
}: {
  provider: ProviderStatus;
  kind: ServiceKind;
  open: boolean;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const service = provider[kind];
  if (service === undefined) return null;
  const name = t(`engines.provider.${provider.id}`);
  return (
    <BottomSheet
      open={open}
      close={onClose}
      maxHeight={0.9}
      testID="provider-sheet"
      header={
        <View
          style={{
            alignSelf: "stretch",
            flexDirection: "row",
            alignItems: "center",
            gap: 12,
            paddingHorizontal: 24,
            paddingTop: 12,
          }}>
          <ProviderIcon id={provider.id} active={service.active} size={44} />
          <View style={{ flex: 1, gap: 4 }}>
            <Text variant="titleLarge" accessibilityRole="header" numberOfLines={2}>
              {name}
            </Text>
            <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 6 }}>
              {providerBadges(service, t).map((b) => (
                <Tag key={b.text} tone={b.tone}>
                  {b.text}
                </Tag>
              ))}
            </View>
          </View>
        </View>
      }>
      <ScrollView
        keyboardShouldPersistTaps="handled"
        contentContainerStyle={{ paddingHorizontal: 24, paddingTop: 8, paddingBottom: 8, gap: 16 }}>
        <Hint>{t(`engines.providerNote.${provider.id}`)}</Hint>
        {provider.id === "builtin" ? (
          <BuiltinBody provider={provider} kind={kind} service={service} onClose={onClose} />
        ) : (
          <ProviderForm provider={provider} kind={kind} service={service} onClose={onClose} />
        )}
      </ScrollView>
    </BottomSheet>
  );
}

function BuiltinBody({
  provider,
  kind,
  service,
  onClose,
}: {
  provider: ProviderStatus;
  kind: ServiceKind;
  service: ServiceStatus;
  onClose: () => void;
}) {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const state = useUiState();
  const probe = useProviderProbe(provider.id, kind);
  const use = useSwitch(provider, kind);
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
    <View style={{ gap: 16 }}>
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
      <ProbePanel
        probe={probe}
        onRun={() => {
          probe.run({});
        }}
      />
      {!service.active && (
        <Button
          mode="contained"
          testID="provider-use"
          onPress={() => {
            use();
            onClose();
          }}>
          {t("engines.setCurrent")}
        </Button>
      )}
    </View>
  );
}

function ProviderForm({
  provider,
  kind,
  service,
  onClose,
}: {
  provider: ProviderStatus;
  kind: ServiceKind;
  service: ServiceStatus;
  onClose: () => void;
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

  /** Checks the draft and writes it, with `provider` serving `kind` when `use`; false when the
   *  checks refused it. The key goes to the keystore only. */
  const write = (use: boolean): string | false => {
    const issue = checkProviderDraft(draft, provider, kind, t);
    setProblem(issue);
    if (issue !== undefined) return false;
    const engines = applyProviderDraft(state.settings.engines, provider.id, kind, draft);
    void backend.invoke("settings_set_engines", {
      engines: use ? withProvider(engines, kind, provider.id) : engines,
    });
    const key = draft.key.trim();
    if (key.length > 0)
      void backend.invoke("provider_key_set", { provider: provider.id, kind, value: key });
    setDraft({ ...draft, key: "" });
    return key.length > 0 ? t("engines.savedKey") : "";
  };
  const save = () => {
    const keyNote = write(false);
    if (keyNote !== false) shell.toast(`${t("engines.saved", { provider: name })}${keyNote}`);
  };
  const saveAndUse = () => {
    const keyNote = write(true);
    if (keyNote === false) return;
    shell.toast(`${t("engines.used", { provider: name })}${keyNote}`);
    onClose();
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
    <View style={{ gap: 16 }} testID="provider-form">
      <View style={{ gap: 8 }}>
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
            testID="provider-model"
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
            testID="provider-model-id"
            onChangeText={(model) => {
              setDraft({ ...draft, model });
            }}
          />
        )}
        <Hint>{t("engines.field.modelHelp")}</Hint>
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
      <View style={{ gap: 4 }}>
        <TextInput
          mode="outlined"
          label={t("engines.field.baseUrl")}
          value={draft.baseUrl}
          placeholder={service.default_base_url ?? "http://127.0.0.1:8000/v1"}
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType="url"
          testID="provider-url"
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
            testID="provider-key"
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
            {service.key.source === "user" && (
              <Button
                compact
                icon="trash-can-outline"
                textColor={theme.colors.error}
                onPress={deleteKey}
                testID="provider-delete-key">
                {t("engines.deleteKey")}
              </Button>
            )}
          </View>
        </View>
      )}
      <ProbePanel
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
      <View style={{ gap: 8 }}>
        <Button compact onPress={reset} style={{ alignSelf: "flex-end" }} testID="provider-reset">
          {t("engines.reset")}
        </Button>
        <View style={{ flexDirection: "row", gap: 12 }}>
          <Button
            mode={service.active ? "contained" : "outlined"}
            onPress={save}
            testID="provider-save"
            style={{ flex: 1 }}>
            {t("engines.save")}
          </Button>
          {!service.active && (
            <Button mode="contained" onPress={saveAndUse} testID="provider-use" style={{ flex: 1 }}>
              {t("engines.setCurrent")}
            </Button>
          )}
        </View>
      </View>
    </View>
  );
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
