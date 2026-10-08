import {
  type FallbackModel,
  type FallbackRowView,
  type FallbackSettings,
  type ProviderId,
  type ServiceKind,
  MAX_FALLBACK_MODELS,
  fallbackAddProblem,
  fallbackRows,
  fallbackSettingsOf,
  fallbackStatusOf,
  formatDateTime,
  modelChoices,
  moveFallback,
  providersFor,
  withFallback,
} from "@voltip/shared";
import { type ReactNode, useState } from "react";
import { useBackend, useUiState } from "../../backend/BackendProvider";
import { Badge } from "../../components/Badge";
import { Button } from "../../components/Button";
import { IconButton } from "../../components/IconButton";
import { Input } from "../../components/Input";
import { type LampTone } from "../../components/Lamp";
import { LampText } from "../../components/LampText";
import { Select } from "../../components/Select";
import { SettingsSection } from "../../components/SettingsLayout";
import { Toggle } from "../../components/Toggle";
import { useI18n } from "../../i18n/I18nProvider";
import { useFeatureShell } from "../shell";

/** The model select's last option: type any other model id. */
const OTHER_MODEL = "__other__";

const TONE: Readonly<Record<FallbackRowView["state"], LampTone>> = {
  active: "ok",
  ready: "neutral",
  exhausted: "warn",
  issue: "warn",
  same: "off",
  duplicate: "off",
};

/** A model id, or a dash for none. */
function shown(model: string): string {
  return model.length > 0 ? model : "—";
}

export interface FallbackSectionProps {
  kind: ServiceKind;
  /** Extra classes for the switch (the phone's touch size). */
  toggleClassName?: string;
}

/** 额度用完后改用其他模型 (docs/dictation.md §3.5) for one service: the switch, the chain — the
 *  selected model first, then the fallback models in order with what became of each (in use,
 *  available, out of quota until a time, a provider problem, skipped) — moving and removing them,
 *  adding one (a provider offering the service, one of its models), 重新检查 once a model ran out,
 *  and Model Studio's note on its 免费额度用完即停. Every change goes through
 *  `settings_set_engines`; the desktop's two engines pages and the phone's use it. */
export function FallbackSection({ kind, toggleClassName }: FallbackSectionProps) {
  const { backend } = useBackend();
  const { notify } = useFeatureShell();
  const { t, locale } = useI18n();
  const state = useUiState();
  const settings = state.settings.engines;
  const config = fallbackSettingsOf(settings, kind);
  const status = fallbackStatusOf(state.engines, kind);
  const { rows } = fallbackRows(state.engines, kind);
  const selected = rows.find((r) => r.index === "selected");
  // The rows follow the core's resolution of the saved list; until it answers a change, an entry
  // shows without its state rather than another entry's.
  const resolved = (i: number): FallbackRowView | undefined => {
    const row = rows.find((r) => r.index === i);
    return row !== undefined && row.provider === config.models[i]?.provider ? row : undefined;
  };
  const save = (next: FallbackSettings) => {
    void backend.invoke("settings_set_engines", {
      engines: withFallback(settings, kind, next),
    });
  };
  const name = (provider: ProviderId) => t(`engines.provider.${provider}`);
  const label = (row: FallbackRowView): string => {
    switch (row.state) {
      case "exhausted":
        return t("engines.fallback.state.exhausted", {
          time: formatDateTime(locale, row.retryAtMs ?? 0, {
            month: "numeric",
            day: "numeric",
            hour: "2-digit",
            minute: "2-digit",
          }),
        });
      case "issue":
        return t(`engines.issue.${row.issue ?? "unavailable"}`);
      default:
        return t(`engines.fallback.state.${row.state}`);
    }
  };
  const notInUse =
    config.enabled && !status.in_use
      ? kind === "asr" && state.engines.asr_provider === "local"
        ? "local"
        : "notReady"
      : undefined;
  const exhausted = rows.some((r) => r.state === "exhausted");
  const studio = rows.some((r) => r.provider === "aliyun");

  return (
    <SettingsSection
      title={t("engines.fallback.title")}
      description={t(`engines.fallback.description.${kind}`)}
      data-testid={`fallback-${kind}`}
      data={{
        "data-enabled": String(config.enabled),
        "data-in-use": String(status.in_use),
      }}
      aside={
        <Toggle
          checked={config.enabled}
          ariaLabel={t("engines.fallback.toggle")}
          label={config.enabled ? t("engines.fallback.on") : t("engines.fallback.off")}
          className={toggleClassName}
          onChange={(enabled) => {
            save({ ...config, enabled });
          }}
        />
      }>
      {notInUse !== undefined && (
        <p
          className="text-[12px] leading-5 text-fg-muted"
          data-testid={`fallback-${kind}-not-in-use`}>
          {t(`engines.fallback.notInUse.${notInUse}`)}
        </p>
      )}
      <ol
        className="flex flex-col divide-y divide-border rounded-md border border-border"
        aria-label={t("engines.fallback.listLabel")}
        data-testid={`fallback-${kind}-list`}>
        {selected !== undefined && (
          <Row
            n={1}
            provider={name(selected.provider)}
            model={shown(selected.model)}
            badge={t("engines.fallback.selected")}
            tone={TONE[selected.state]}
            status={label(selected)}
            state={selected.state}
          />
        )}
        {config.models.map((entry, i) => {
          const row = resolved(i);
          const model = shown(row?.model ?? entry.model);
          return (
            <Row
              key={`${entry.provider}:${entry.model}:${String(i)}`}
              n={i + (selected === undefined ? 1 : 2)}
              provider={name(entry.provider)}
              model={model}
              tone={row === undefined ? "idle" : TONE[row.state]}
              status={row === undefined ? undefined : label(row)}
              state={row?.state}
              actions={
                <>
                  <IconButton
                    icon="chevronUp"
                    label={t("engines.fallback.moveUp", { model })}
                    disabled={i === 0}
                    onClick={() => {
                      save({
                        ...config,
                        models: moveFallback(config.models, i, -1),
                      });
                    }}
                  />
                  <IconButton
                    icon="chevronDown"
                    label={t("engines.fallback.moveDown", { model })}
                    disabled={i === config.models.length - 1}
                    onClick={() => {
                      save({
                        ...config,
                        models: moveFallback(config.models, i, 1),
                      });
                    }}
                  />
                  <IconButton
                    icon="trash"
                    tone="danger"
                    label={t("engines.fallback.remove", { model })}
                    onClick={() => {
                      save({
                        ...config,
                        models: config.models.filter((_, j) => j !== i),
                      });
                    }}
                  />
                </>
              }
            />
          );
        })}
      </ol>
      {config.models.length === 0 && (
        <p className="text-[12px] text-fg-muted" data-testid={`fallback-${kind}-empty`}>
          {t("engines.fallback.empty")}
        </p>
      )}
      <AddFallback
        kind={kind}
        list={config.models}
        onAdd={(entry) => {
          save({ ...config, models: [...config.models, entry] });
        }}
      />
      {(exhausted || studio) && (
        <div className="flex flex-col items-start gap-2">
          {exhausted && (
            <Button
              size="sm"
              variant="outline"
              data-testid={`fallback-${kind}-recheck`}
              onClick={() => {
                void backend.invoke("engines_quota_reset", { kind });
                notify(t("engines.fallback.recheckDone"));
              }}>
              {t("engines.fallback.recheck")}
            </Button>
          )}
          {studio && (
            <p
              className="text-[12px] leading-5 text-fg-muted"
              data-testid={`fallback-${kind}-aliyun`}>
              {t("engines.fallback.aliyunNote")}
            </p>
          )}
        </div>
      )}
    </SettingsSection>
  );
}

function Row({
  n,
  provider,
  model,
  badge,
  tone,
  status,
  state,
  actions,
}: {
  n: number;
  provider: string;
  model: string;
  badge?: string;
  tone: LampTone;
  status?: string;
  state?: FallbackRowView["state"];
  actions?: ReactNode;
}) {
  return (
    <li
      className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2"
      data-state={state ?? "pending"}>
      <span className="mono w-4 text-[12px] text-fg-subtle">{n}</span>
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1">
        <span className="text-[13px] text-fg">{provider}</span>
        <span className="mono min-w-0 truncate text-[12px] text-fg-muted">{model}</span>
        {badge !== undefined && <Badge>{badge}</Badge>}
      </div>
      {status !== undefined && (
        <LampText tone={tone} size="sm">
          {status}
        </LampText>
      )}
      {actions !== undefined && <div className="flex items-center gap-1">{actions}</div>}
    </li>
  );
}

/** The add row: a provider offering the service (not the on-device one), one of its models — the
 *  card's presets or any id — and 添加, refused with the reason when the list cannot take it. */
function AddFallback({
  kind,
  list,
  onAdd,
}: {
  kind: ServiceKind;
  list: readonly FallbackModel[];
  onAdd: (entry: FallbackModel) => void;
}) {
  const { t } = useI18n();
  const state = useUiState();
  const candidates = providersFor(state.engines, kind).filter((p) => p.id !== "local");
  const [providerId, setProviderId] = useState<ProviderId | undefined>(undefined);
  const provider = candidates.find((p) => p.id === providerId) ?? candidates[0];
  const service = provider?.[kind];
  const choices = service === undefined ? [] : modelChoices(service);
  const [picked, setPicked] = useState<string | undefined>(undefined);
  const [typed, setTyped] = useState("");
  const [problem, setProblem] = useState<string | undefined>(undefined);
  if (provider === undefined || service === undefined) return null;
  const builtin = provider.id === "builtin";
  // A built-in service with several models offers them, no other id; with one, it is the card's.
  const builtinChoice = builtin && choices.length > 1;
  const selectValue = picked ?? choices[0] ?? OTHER_MODEL;
  const model = builtin
    ? builtinChoice
      ? selectValue
      : ""
    : selectValue === OTHER_MODEL
      ? typed.trim()
      : selectValue;
  const add = () => {
    const entry: FallbackModel = { provider: provider.id, model };
    const why = fallbackAddProblem(list, entry);
    if (why !== undefined) {
      setProblem(t(`engines.fallback.problem.${why}`, { n: MAX_FALLBACK_MODELS }));
      return;
    }
    setProblem(undefined);
    setTyped("");
    onAdd(entry);
  };
  return (
    <div className="flex flex-col gap-2" data-testid={`fallback-${kind}-add`}>
      <div className="grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(200px,1fr))]">
        <Select
          label={t("engines.fallback.provider")}
          size="sm"
          value={provider.id}
          onChange={(id) => {
            setProviderId(id);
            setPicked(undefined);
            setTyped("");
            setProblem(undefined);
          }}
          options={candidates.map((p) => ({
            value: p.id,
            label: t(`engines.provider.${p.id}`),
          }))}
        />
        {builtinChoice ? (
          <Select
            label={t("engines.fallback.model")}
            size="sm"
            mono
            value={selectValue}
            onChange={(value) => {
              setPicked(value);
              setProblem(undefined);
            }}
            options={choices.map((m) => ({ value: m, label: m }))}
          />
        ) : builtin ? (
          <Input
            label={t("engines.fallback.model")}
            size="sm"
            mono
            value={service.model}
            readOnly
          />
        ) : (
          <>
            {choices.length > 0 && (
              <Select
                label={t("engines.fallback.model")}
                size="sm"
                mono
                value={selectValue}
                onChange={(value) => {
                  setPicked(value);
                  setProblem(undefined);
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
                size="sm"
                mono
                value={typed}
                onChange={(e) => {
                  setTyped(e.target.value);
                  setProblem(undefined);
                }}
              />
            )}
          </>
        )}
      </div>
      <div className="flex items-center gap-3">
        <Button
          size="sm"
          variant="outline"
          onClick={add}
          data-testid={`fallback-${kind}-add-button`}>
          {t("engines.fallback.add")}
        </Button>
        {problem !== undefined && (
          <span role="alert" className="text-[12px] text-danger">
            {problem}
          </span>
        )}
      </div>
    </div>
  );
}
