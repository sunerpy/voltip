import {
  type ProviderId,
  type ProviderStatus,
  type ServiceKind,
  type ServiceStatus,
  REASONING_EFFORTS,
  type ReasoningEffort,
  applyProviderDraft,
  checkProviderDraft,
  choosesInterface,
  modelDisplayName,
  modelListHelp,
  modelOptions,
  probeText,
  providerBadges,
  providerDraft,
  providerTagline,
  secretStateLabel,
  withProvider,
} from "@voltip/shared";
import { type ReactNode, useEffect, useState } from "react";
import { useBackend, useUiState } from "../../backend/BackendProvider";
import { Badge } from "../../components/Badge";
import { Button } from "../../components/Button";
import { Drawer, DrawerBody, DrawerFooter } from "../../components/Drawer";
import { Icon, type IconName } from "../../components/Icon";
import { IconButton } from "../../components/IconButton";
import { Input } from "../../components/Input";
import { LampText } from "../../components/LampText";
import { Select } from "../../components/Select";
import { CardGrid } from "../../components/SettingsLayout";
import { cx } from "../../cx";
import { useI18n } from "../../i18n/I18nProvider";
import { useFeatureShell } from "../shell";
import { ProviderMark, hasProviderMark } from "./ProviderMark";
import { useModelRefresh } from "./useModelRefresh";
import { useProviderProbe } from "./useProviderProbe";

/** The model select's last option: type any other model id. */
const OTHER_MODEL = "__other__";

/** The effort select's "not set" choice (no `reasoning` in the request). */
const NO_EFFORT = "";

/** The app's own icon of each provider: the built-in, on-device and custom ones always, a vendor
 *  when it has no mark of its own (`PROVIDER_MARKS`). */
const ICONS: Readonly<Record<ProviderId, IconName>> = {
  builtin: "sparkles",
  local: "cpu",
  openai: "cloud",
  groq: "cloud",
  google: "cloud",
  siliconflow: "cloud",
  aliyun: "cloud",
  deepseek: "cloud",
  ollama: "monitor",
  custom: "link",
};

/** How wide a provider's drawer gets: the on-device models need two columns of model cards. */
const DRAWER_WIDTH = 480;
const LOCAL_DRAWER_WIDTH = 720;

export interface ProviderCardsProps {
  kind: ServiceKind;
  providers: readonly ProviderStatus[];
  /** The grid's name: the section's title. */
  label: string;
  /** The on-device provider's settings (the desktop's model library); a shell without local
   *  models never lists that provider. */
  localBody?: ReactNode;
}

/** The providers of one service (docs/dictation.md §3) as a grid of cards (user decision
 *  2026-10-10, Penpot 「Voltip 服务商设置」 option 1): each card names the provider, where its
 *  requests go, whether it can run and the model in effect, with 设为当前 to switch to it. A click
 *  on a card opens its settings in a drawer: the built-in service explains itself, the on-device
 *  provider holds the shell's `localBody`, every other one is the form for its model, endpoint and
 *  key, one field per row, with 测试连接. The desktop's two model pages and the phone's webview
 *  settings both use it. */
export function ProviderCards({ kind, providers, label, localBody }: ProviderCardsProps) {
  const [openId, setOpenId] = useState<ProviderId | undefined>(undefined);
  const open = providers.find((p) => p.id === openId);
  return (
    <>
      <CardGrid min={280} role="list" aria-label={label} className="gap-3">
        {providers.map((p) => (
          <div role="listitem" key={p.id} className="flex min-w-0">
            <ProviderTile
              provider={p}
              kind={kind}
              open={p.id === openId}
              onOpen={() => {
                setOpenId(p.id);
              }}
            />
          </div>
        ))}
      </CardGrid>
      {open !== undefined && (
        <ProviderDetail
          key={`${open.id}-${kind}`}
          provider={open}
          kind={kind}
          localBody={localBody}
          onClose={() => {
            setOpenId(undefined);
          }}
        />
      )}
    </>
  );
}

/** The provider's icon on a block: a soft accent one for the provider in use, never a solid one
 *  (user feedback 2026-10-10 on the drafts). A vendor shows its own mark in its own colours (user
 *  request 2026-10-12); the built-in, on-device and custom providers keep the app's icons. */
function ProviderIcon({ id, active }: { id: ProviderId; active: boolean }) {
  const mark = hasProviderMark(id);
  return (
    <span
      className={cx(
        "flex size-8 shrink-0 items-center justify-center rounded-6",
        active ? "bg-accent-soft" : "bg-inset",
        mark ? "text-fg" : active ? "text-accent-text" : "text-fg-muted",
      )}>
      {mark ? <ProviderMark id={id} size={18} /> : <Icon name={ICONS[id]} size={16} />}
    </span>
  );
}

function StateBadges({ service }: { service: ServiceStatus }) {
  const { t } = useI18n();
  return (
    <>
      {providerBadges(service, t).map((b) => (
        <Badge key={b.text} tone={b.tone}>
          {b.text}
        </Badge>
      ))}
    </>
  );
}

/** Switches `kind` to `provider`. */
function useSwitch(provider: ProviderStatus, kind: ServiceKind) {
  const { backend } = useBackend();
  const { notify } = useFeatureShell();
  const { t } = useI18n();
  const state = useUiState();
  return () => {
    void backend.invoke("settings_set_engines", {
      engines: withProvider(state.settings.engines, kind, provider.id),
    });
    notify(t("engines.used", { provider: t(`engines.provider.${provider.id}`) }));
  };
}

/** One card: the whole card opens the drawer (its name is the button, stretched over the card),
 *  and 设为当前 sits above that. */
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
  const { t, locale } = useI18n();
  const state = useUiState();
  const use = useSwitch(provider, kind);
  const service = provider[kind];
  if (service === undefined) return null;
  const name = t(`engines.provider.${provider.id}`);
  const model =
    provider.id === "local"
      ? modelDisplayName(
          service.model,
          state.models.find((m) => m.id === service.model)?.name ?? service.model,
          locale,
        )
      : service.model;
  return (
    <article
      aria-label={name}
      data-testid={`provider-${kind}-${provider.id}`}
      data-selected={service.active || undefined}
      data-open={open || undefined}
      className={cx(
        "relative flex min-w-0 flex-1 flex-col gap-3 rounded-10 border p-4 transition-colors",
        service.active
          ? "border-accent/40 bg-accent-soft/40"
          : "border-border bg-surface hover:border-fg-subtle",
        open && "inset-ring-2 inset-ring-accent",
      )}>
      <div className="flex items-start gap-3">
        <ProviderIcon id={provider.id} active={service.active} />
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <button
            type="button"
            aria-haspopup="dialog"
            aria-expanded={open}
            onClick={onOpen}
            className="truncate text-left text-[14px] leading-5 font-semibold text-fg outline-none after:absolute after:inset-0 after:rounded-10 focus-visible:after:inset-ring-2 focus-visible:after:inset-ring-accent">
            {name}
          </button>
          <span className="truncate text-[12px] leading-4 text-fg-muted">
            {providerTagline(provider, kind, t)}
          </span>
        </div>
        <span className="flex shrink-0 flex-wrap items-center justify-end gap-1">
          <StateBadges service={service} />
        </span>
      </div>
      <div className="flex items-end gap-3 border-t border-border pt-3">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-[11px] leading-4 text-fg-subtle">{t("engines.field.model")}</span>
          <span
            className={cx(
              "mono truncate text-[12px] leading-5",
              model.length > 0 ? "text-fg" : "text-fg-subtle",
            )}
            data-testid="provider-model">
            {model.length > 0 ? model : "—"}
          </span>
        </div>
        {service.active ? (
          <span className="shrink-0 text-[12px] leading-7 font-medium text-accent-text">
            {t("engines.isCurrent")}
          </span>
        ) : (
          <Button
            size="sm"
            variant="text"
            className="relative z-10"
            onClick={use}
            title={t("engines.useTitle", {
              provider: name,
              service: t(`engines.service.${kind}`),
            })}>
            {t("engines.setCurrent")}
          </Button>
        )}
      </div>
    </article>
  );
}

export interface ProviderDetailProps {
  provider: ProviderStatus;
  kind: ServiceKind;
  onClose: () => void;
  localBody?: ReactNode;
}

/** One provider's settings for one service, in the drawer a card opens. */
export function ProviderDetail({ provider, kind, onClose, localBody }: ProviderDetailProps) {
  const { t } = useI18n();
  const service = provider[kind];
  if (service === undefined) return null;
  const name = t(`engines.provider.${provider.id}`);
  return (
    <Drawer
      open
      onClose={onClose}
      title={name}
      icon={ICONS[provider.id]}
      {...(hasProviderMark(provider.id)
        ? { mark: <ProviderMark id={provider.id} size={20} /> }
        : {})}
      iconTone={service.active ? "accent" : "neutral"}
      badges={<StateBadges service={service} />}
      description={t(`engines.providerNote.${provider.id}`)}
      width={provider.id === "local" ? LOCAL_DRAWER_WIDTH : DRAWER_WIDTH}
      data-testid={`provider-${kind}-${provider.id}-detail`}>
      {provider.id === "builtin" ? (
        <BuiltinDetail provider={provider} kind={kind} service={service} onClose={onClose} />
      ) : provider.id === "local" ? (
        <>
          <DrawerBody>{localBody}</DrawerBody>
          <UseFooter provider={provider} kind={kind} service={service} onClose={onClose} />
        </>
      ) : (
        <ProviderForm provider={provider} kind={kind} service={service} onClose={onClose} />
      )}
    </Drawer>
  );
}

/** 设为当前 alone, for a provider with nothing to save; nothing for the one in use. */
function UseFooter({
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
  const { t } = useI18n();
  const use = useSwitch(provider, kind);
  if (service.active) return null;
  return (
    <DrawerFooter className="justify-end">
      <Button
        variant="primary"
        onClick={() => {
          use();
          onClose();
        }}>
        {t("engines.setCurrent")}
      </Button>
    </DrawerFooter>
  );
}

/** The eyebrow over one group of fields. */
function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-3">
      <h3 className="eyebrow">{title}</h3>
      {children}
    </section>
  );
}

function Help({ children, ...rest }: { children: ReactNode; "data-testid"?: string }) {
  return (
    <p className="text-[12px] leading-5 text-fg-muted" data-testid={rest["data-testid"]}>
      {children}
    </p>
  );
}

function BuiltinDetail({
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
  const { notify } = useFeatureShell();
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
    notify(t("engines.saved", { provider: t(`engines.provider.${provider.id}`) }));
  };
  return (
    <>
      <DrawerBody>
        <Group title={t(`engines.service.${kind}`)}>
          <Help>{t("engines.builtinBody")}</Help>
          {service.presets.length > 1 ? (
            <Select
              label={t("engines.field.model")}
              mono
              wrap
              value={service.model}
              onChange={choose}
              options={service.presets.map((m) => ({ value: m, label: m }))}
              data-testid={`builtin-${kind}-model`}
            />
          ) : (
            <div className="mono text-[12px] break-all text-fg">
              {t("engines.model", { model: service.model })}
            </div>
          )}
          <ProbePanel
            probe={probe}
            onRun={() => {
              probe.run({});
            }}
          />
        </Group>
      </DrawerBody>
      <UseFooter provider={provider} kind={kind} service={service} onClose={onClose} />
    </>
  );
}

/** 测试连接 and what it answered, or that it has not run yet. */
function ProbePanel({
  probe,
  onRun,
}: {
  probe: ReturnType<typeof useProviderProbe>;
  onRun: () => void;
}) {
  const { t } = useI18n();
  const result = probe.report ? probeText(probe.report, t) : undefined;
  return (
    <div className="flex flex-wrap items-center gap-3 rounded-10 bg-inset px-3 py-2.5">
      <Button size="sm" variant="outline" icon="refresh" loading={probe.pending} onClick={onRun}>
        {probe.pending ? t("engines.probing") : t("engines.probe")}
      </Button>
      {result !== undefined && !probe.pending ? (
        <span role="status" data-testid="probe-result">
          <LampText tone={result.ok ? "ok" : "danger"} size="sm">
            {result.text}
          </LampText>
        </span>
      ) : (
        !probe.pending && (
          <span className="text-[12px] text-fg-subtle">{t("engines.probeIdle")}</span>
        )
      )}
    </div>
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
  const options = modelOptions(service, probed, t, {
    value: OTHER_MODEL,
    label: t("engines.field.modelOther"),
  });
  const choices = options.filter((o) => o.value !== OTHER_MODEL).map((o) => o.value);
  const effectiveModel = draft.model.length > 0 ? draft.model : service.model;
  const [typing, setTyping] = useState(() => choices.length === 0);
  const selectValue = typing || !choices.includes(effectiveModel) ? OTHER_MODEL : effectiveModel;
  const takesKey = provider.key !== "none";
  const shared =
    provider.asr !== undefined && provider.llm !== undefined && provider.id !== "custom";
  const secret = secretStateLabel(service.key, locale);
  // docs/dictation.md §3.3: the provider's latest models as the drawer opens, and again each time
  // the model list opens.
  const refreshModels = useModelRefresh(provider.id, kind);
  useEffect(() => {
    refreshModels();
  }, [refreshModels]);

  /** Checks the draft and writes it, with `provider` serving `kind` when `use`; false when the
   *  checks refused it. The key goes to the keychain only. */
  const write = (use: boolean): string | false => {
    const issue = checkProviderDraft(draft, provider, kind, t);
    setProblem(issue);
    if (issue !== undefined) return false;
    const engines = applyProviderDraft(state.settings.engines, provider.id, kind, draft);
    const sent = [
      backend.invoke("settings_set_engines", {
        engines: use ? withProvider(engines, kind, provider.id) : engines,
      }),
    ];
    const key = draft.key.trim();
    if (key.length > 0)
      sent.push(backend.invoke("provider_key_set", { provider: provider.id, kind, value: key }));
    // The list for the endpoint and key just saved, once the core has them.
    Promise.all(sent).then(refreshModels, () => undefined);
    setDraft({ ...draft, key: "" });
    return key.length > 0 ? t("engines.savedKey") : "";
  };
  const save = () => {
    const keyNote = write(false);
    if (keyNote !== false) notify(`${t("engines.saved", { provider: name })}${keyNote}`);
  };
  const saveAndUse = () => {
    const keyNote = write(true);
    if (keyNote === false) return;
    notify(`${t("engines.used", { provider: name })}${keyNote}`);
    onClose();
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
    setDraft(
      choosesInterface(provider.id, kind)
        ? { model: "", baseUrl: "", key: "", api: "chat_completions" }
        : { model: "", baseUrl: "", key: "" },
    );
    setTyping(false);
    setProblem(undefined);
    notify(t("engines.resetDone", { provider: name }));
  };

  return (
    // Fields and actions alike: the tests look for both inside it.
    <div className="flex min-h-0 flex-1 flex-col" data-testid="provider-form">
      <DrawerBody>
        <Group title={t(`engines.service.${kind}`)}>
          {choices.length > 0 && (
            <Select
              label={t("engines.field.model")}
              mono
              wrap
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
              onOpen={refreshModels}
              options={options}
              data-testid="provider-model-select"
            />
          )}
          {selectValue === OTHER_MODEL && (
            <Input
              label={t("engines.field.modelCustom")}
              mono
              value={draft.model}
              placeholder={service.model}
              onChange={(e) => {
                setDraft({ ...draft, model: e.target.value });
              }}
            />
          )}
          <Help data-testid="provider-model-help">{modelListHelp(provider, service, t)}</Help>
          {choosesInterface(provider.id, kind) && (
            // docs/dictation.md §3.7: the custom clean-up's interface, and the Responses effort.
            <>
              <Select
                label={t("engines.field.api")}
                value={draft.api ?? "chat_completions"}
                onChange={(value) => {
                  setDraft({
                    ...draft,
                    api: value === "responses" ? "responses" : "chat_completions",
                  });
                }}
                options={[
                  { value: "chat_completions", label: "Chat Completions" },
                  { value: "responses", label: "Responses" },
                ]}
              />
              {draft.api === "responses" && (
                <Select
                  label={t("engines.field.reasoning")}
                  value={draft.reasoning ?? NO_EFFORT}
                  onChange={(value) => {
                    const reasoning = REASONING_EFFORTS.find((e) => e === value);
                    const next = { ...draft };
                    if (reasoning === undefined) delete next.reasoning;
                    else next.reasoning = reasoning;
                    setDraft(next);
                  }}
                  options={[
                    { value: NO_EFFORT, label: t("engines.field.reasoningNone") },
                    ...REASONING_EFFORTS.map((e: ReasoningEffort) => ({ value: e, label: e })),
                  ]}
                />
              )}
              <Help data-testid="provider-api-help">
                {t("engines.field.apiHelp")}
                {draft.api === "responses" ? ` ${t("engines.field.reasoningHelp")}` : ""}
              </Help>
            </>
          )}
        </Group>
        <Group title={t("engines.detail.connection")}>
          <Input
            label={t("engines.field.baseUrl")}
            mono
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
          {takesKey && (
            <>
              <Input
                label={
                  provider.key === "required"
                    ? t("engines.field.key")
                    : t("engines.field.keyOptional")
                }
                mono
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
                help={
                  shared
                    ? t("engines.field.keyShared", { provider: name })
                    : t("engines.field.keyHelp")
                }
                trailing={
                  <IconButton
                    icon="eye"
                    label={showKey ? t("engines.field.hideKey") : t("engines.field.showKey")}
                    onClick={() => {
                      setShowKey(!showKey);
                    }}
                  />
                }
              />
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px]">
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
                {provider.quota === true && (
                  <Button
                    size="sm"
                    variant="text"
                    icon="external"
                    data-testid="provider-quota"
                    onClick={() => {
                      void backend.providerConsoleOpen(provider.id, "quota");
                    }}>
                    {t("engines.viewQuota")}
                  </Button>
                )}
                {service.key.source === "user" && (
                  <Button size="sm" variant="text-danger" icon="trash" onClick={deleteKey}>
                    {t("engines.deleteKey")}
                  </Button>
                )}
              </div>
              {provider.quota === true && (
                <Help data-testid="provider-quota-help">{t("engines.quotaHelp")}</Help>
              )}
            </>
          )}
          <ProbePanel
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
        </Group>
      </DrawerBody>
      <DrawerFooter>
        <Button variant="ghost" onClick={reset}>
          {t("engines.reset")}
        </Button>
        <div className="ml-auto flex items-center gap-2">
          <Button variant={service.active ? "primary" : "outline"} onClick={save}>
            {t("engines.save")}
          </Button>
          {!service.active && (
            <Button variant="primary" onClick={saveAndUse}>
              {t("engines.setCurrent")}
            </Button>
          )}
        </div>
      </DrawerFooter>
    </div>
  );
}
