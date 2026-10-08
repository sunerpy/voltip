// 额度用完后改用其他模型 (docs/dictation.md §3.5), `@voltip/ui`'s FallbackSection on native views:
// the switch, the chain — the selected model first, then the fallback models in order with what
// became of each — moving and removing them, adding one, 重新检查 once a model ran out, and Model
// Studio's note on its 免费额度用完即停. Every change goes through `settings_set_engines`.
import {
  type FallbackModel,
  type FallbackRowView,
  type FallbackSettings,
  MAX_FALLBACK_MODELS,
  type ProviderId,
  type ServiceKind,
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
import { useState } from "react";
import { View } from "react-native";
import { IconButton, Text, TextInput } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import {
  Hint,
  Mono,
  RowDivider,
  Section,
  StateLine,
  SwitchRow,
  type Tone,
  useAppTheme,
} from "../ui/kit";
import { SelectField } from "../ui/Select";
import { OTHER_MODEL } from "./engines";

const TONE: Readonly<Record<FallbackRowView["state"], Tone>> = {
  active: "ok",
  ready: "idle",
  exhausted: "warning",
  issue: "warning",
  same: "idle",
  duplicate: "idle",
};

function shown(model: string): string {
  return model.length > 0 ? model : "—";
}

function ChainRow({
  n,
  provider,
  model,
  badge,
  tone,
  status,
  actions,
}: {
  n: number;
  provider: string;
  model: string;
  badge?: string;
  tone: Tone;
  status?: string | undefined;
  actions?: React.ReactNode;
}) {
  const theme = useAppTheme();
  return (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: 12,
        paddingLeft: 16,
        paddingRight: 4,
        paddingVertical: 8,
        minHeight: 56,
      }}>
      <Mono style={{ width: 16 }}>{n}</Mono>
      <View style={{ flex: 1, gap: 2 }}>
        <Text variant="bodyLarge">
          {provider}
          {badge !== undefined && (
            <Text variant="labelSmall" style={{ color: theme.colors.primary }}>{`  ${badge}`}</Text>
          )}
        </Text>
        <Mono numberOfLines={1}>{model}</Mono>
        {status !== undefined && (
          <StateLine tone={tone} small>
            {status}
          </StateLine>
        )}
      </View>
      {actions}
    </View>
  );
}

function AddFallback({
  kind,
  list,
  onAdd,
}: {
  kind: ServiceKind;
  list: readonly FallbackModel[];
  onAdd: (entry: FallbackModel) => void;
}) {
  const theme = useAppTheme();
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
    <View style={{ gap: 12 }} testID={`fallback-${kind}-add`}>
      <SelectField
        label={t("engines.fallback.provider")}
        value={provider.id}
        onChange={(id) => {
          setProviderId(id);
          setPicked(undefined);
          setTyped("");
          setProblem(undefined);
        }}
        options={candidates.map((p) => ({ value: p.id, label: t(`engines.provider.${p.id}`) }))}
      />
      {builtinChoice ? (
        <SelectField
          label={t("engines.fallback.model")}
          value={selectValue}
          onChange={(value) => {
            setPicked(value);
            setProblem(undefined);
          }}
          options={choices.map((m) => ({ value: m, label: m }))}
        />
      ) : builtin ? (
        <TextInput
          mode="outlined"
          label={t("engines.fallback.model")}
          value={service.model}
          editable={false}
        />
      ) : (
        <>
          {choices.length > 0 && (
            <SelectField
              label={t("engines.fallback.model")}
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
            <TextInput
              mode="outlined"
              label={t("engines.field.modelCustom")}
              value={typed}
              autoCapitalize="none"
              autoCorrect={false}
              onChangeText={(text) => {
                setTyped(text);
                setProblem(undefined);
              }}
            />
          )}
        </>
      )}
      <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
        <Button mode="outlined" icon="plus" onPress={add} testID={`fallback-${kind}-add-button`}>
          {t("engines.fallback.add")}
        </Button>
        {problem !== undefined && (
          <Text
            variant="bodySmall"
            accessibilityRole="alert"
            style={{ flex: 1, color: theme.colors.error }}>
            {problem}
          </Text>
        )}
      </View>
    </View>
  );
}

export function FallbackSection({ kind }: { kind: ServiceKind }) {
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const state = useUiState();
  const settings = state.settings.engines;
  const config = fallbackSettingsOf(settings, kind);
  const status = fallbackStatusOf(state.engines, kind);
  const { rows } = fallbackRows(state.engines, kind);
  const selected = rows.find((r) => r.index === "selected");
  const resolved = (i: number): FallbackRowView | undefined => {
    const row = rows.find((r) => r.index === i);
    return row !== undefined && row.provider === config.models[i]?.provider ? row : undefined;
  };
  const save = (next: FallbackSettings) => {
    void backend.invoke("settings_set_engines", { engines: withFallback(settings, kind, next) });
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
    <View style={{ gap: 8 }} testID={`fallback-${kind}`}>
      <Section
        title={t("engines.fallback.title")}
        footer={t(`engines.fallback.description.${kind}`)}>
        <SwitchRow
          title={t("engines.fallback.toggle")}
          description={config.enabled ? t("engines.fallback.on") : t("engines.fallback.off")}
          value={config.enabled}
          onValueChange={(enabled) => {
            save({ ...config, enabled });
          }}
          testID={`fallback-${kind}-toggle`}
        />
        {notInUse !== undefined && (
          <View style={{ paddingHorizontal: 16, paddingBottom: 12 }}>
            <Hint>{t(`engines.fallback.notInUse.${notInUse}`)}</Hint>
          </View>
        )}
        <RowDivider />
        <View accessibilityLabel={t("engines.fallback.listLabel")} testID={`fallback-${kind}-list`}>
          {selected !== undefined && (
            <ChainRow
              n={1}
              provider={name(selected.provider)}
              model={shown(selected.model)}
              badge={t("engines.fallback.selected")}
              tone={TONE[selected.state]}
              status={label(selected)}
            />
          )}
          {config.models.map((entry, i) => {
            const row = resolved(i);
            const model = shown(row?.model ?? entry.model);
            return (
              <View key={`${entry.provider}:${entry.model}:${String(i)}`}>
                <RowDivider />
                <ChainRow
                  n={i + (selected === undefined ? 1 : 2)}
                  provider={name(entry.provider)}
                  model={model}
                  tone={row === undefined ? "idle" : TONE[row.state]}
                  status={row === undefined ? undefined : label(row)}
                  actions={
                    <View style={{ flexDirection: "row" }}>
                      <IconButton
                        icon="arrow-up"
                        accessibilityLabel={t("engines.fallback.moveUp", { model })}
                        disabled={i === 0}
                        onPress={() => {
                          save({ ...config, models: moveFallback(config.models, i, -1) });
                        }}
                      />
                      <IconButton
                        icon="arrow-down"
                        accessibilityLabel={t("engines.fallback.moveDown", { model })}
                        disabled={i === config.models.length - 1}
                        onPress={() => {
                          save({ ...config, models: moveFallback(config.models, i, 1) });
                        }}
                      />
                      <IconButton
                        icon="close"
                        accessibilityLabel={t("engines.fallback.remove", { model })}
                        onPress={() => {
                          save({ ...config, models: config.models.filter((_, j) => j !== i) });
                        }}
                      />
                    </View>
                  }
                />
              </View>
            );
          })}
        </View>
      </Section>
      {config.models.length === 0 && <Hint>{t("engines.fallback.empty")}</Hint>}
      <Section padded>
        <AddFallback
          kind={kind}
          list={config.models}
          onAdd={(entry) => {
            save({ ...config, models: [...config.models, entry] });
          }}
        />
      </Section>
      {exhausted && (
        <Button
          mode="outlined"
          style={{ alignSelf: "flex-start" }}
          testID={`fallback-${kind}-recheck`}
          onPress={() => {
            void backend.invoke("engines_quota_reset", { kind });
            shell.toast(t("engines.fallback.recheckDone"));
          }}>
          {t("engines.fallback.recheck")}
        </Button>
      )}
      {studio && <Hint>{t("engines.fallback.aliyunNote")}</Hint>}
    </View>
  );
}
