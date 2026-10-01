import {
  type AudioDevice,
  type EngineSettings,
  type EngineStatus,
  type Locale,
  type ModelState,
  PROVIDER_IDS,
  RECORDING_SOURCES,
  type RecordingSource,
  type ProviderId,
  type ServiceKind,
  type TFunction,
  isRecognitionModel,
  modelDisplayName,
  modelFamilyName,
  recordingSourceLabel,
  zhT,
} from "@voltip/shared";
import type { MenuItem, MenuSection } from "@voltip/ui";
import { shortMicrophoneName } from "../audio/mic-store";
import {
  activateLocalModel,
  applyProviderDraft,
  modelChoices,
  providersFor,
  withProvider,
} from "../../pages/settings/engines/helpers";
import { shortModel } from "../../shell/page-meta";

/** What a row of the 语音模型, AI 润色模型 or 麦克风 menu stands for (plan 2026-09-30: the title
 *  bar and the home page switch them in place, without the settings dialog). */
export type SwitchChoice =
  /** One model of a cloud provider (or the built-in service). */
  | { kind: "remote"; provider: ProviderId; model: string }
  /** An installed local recognition model. */
  | { kind: "local"; id: string }
  /** An input device; `null` is the system default. */
  | { kind: "microphone"; device: string | null }
  /** What a take records (docs/dictation.md §22). */
  | { kind: "source"; source: RecordingSource }
  /** The command at the end of a menu: open the page (or the settings group) behind it. */
  | { kind: "manage"; target: ManageTarget };

export type ManageTarget = "speech" | "ai" | "microphone";

/** `remote:<provider>:<model>`, `local:<id>`, `mic:` (the system default) or `mic:<id>`,
 *  `manage:<target>`. A model id may contain `:` (`ollama` tags), so it is the rest of the id. */
export function choiceId(choice: SwitchChoice): string {
  switch (choice.kind) {
    case "remote":
      return `remote:${choice.provider}:${choice.model}`;
    case "local":
      return `local:${choice.id}`;
    case "microphone":
      return `mic:${choice.device ?? ""}`;
    case "source":
      return `source:${choice.source}`;
    case "manage":
      return `manage:${choice.target}`;
  }
}

const MANAGE_TARGETS: ReadonlySet<string> = new Set<ManageTarget>(["speech", "ai", "microphone"]);
const PROVIDERS: ReadonlySet<string> = new Set<ProviderId>(PROVIDER_IDS);
const SOURCES: ReadonlySet<string> = new Set<RecordingSource>(RECORDING_SOURCES);

function isProvider(value: string): value is ProviderId {
  return PROVIDERS.has(value);
}

function isSource(value: string): value is RecordingSource {
  return SOURCES.has(value);
}

function isManageTarget(value: string): value is ManageTarget {
  return MANAGE_TARGETS.has(value);
}

/** The choice a row id names, `undefined` for an id these menus did not make. */
export function parseChoice(id: string): SwitchChoice | undefined {
  const colon = id.indexOf(":");
  if (colon < 0) return undefined;
  const kind = id.slice(0, colon);
  const rest = id.slice(colon + 1);
  switch (kind) {
    case "remote": {
      const split = rest.indexOf(":");
      const provider = rest.slice(0, split);
      const model = rest.slice(split + 1);
      return split > 0 && isProvider(provider) && model.length > 0
        ? { kind: "remote", provider, model }
        : undefined;
    }
    case "local":
      return rest.length > 0 ? { kind: "local", id: rest } : undefined;
    case "mic":
      return { kind: "microphone", device: rest.length > 0 ? rest : null };
    case "source":
      return isSource(rest) ? { kind: "source", source: rest } : undefined;
    case "manage":
      return isManageTarget(rest) ? { kind: "manage", target: rest } : undefined;
    default:
      return undefined;
  }
}

/** One section per provider that can run `kind`, its models as rows, the one in use checked; a
 *  provider that cannot run yet is one disabled row in a closing 需要配置 section, with the reason. */
function providerSections(
  status: EngineStatus,
  kind: ServiceKind,
  t: TFunction,
): { ready: MenuSection[]; unavailable: MenuItem[] } {
  const ready: MenuSection[] = [];
  const unavailable: MenuItem[] = [];
  for (const provider of providersFor(status, kind)) {
    if (provider.id === "local") continue;
    const service = provider[kind];
    if (service === undefined) continue;
    const name = t(`engines.provider.${provider.id}`);
    if (service.issue !== undefined) {
      unavailable.push({
        kind: "radio",
        // Never chosen (the row is disabled): an id parseChoice does not accept.
        id: `unavailable:${provider.id}`,
        label: name,
        checked: false,
        disabled: true,
        detail: t(`engines.issue.${service.issue}`),
      });
      continue;
    }
    ready.push({
      label: name,
      items: modelChoices(service).map((model) => ({
        kind: "radio",
        id: choiceId({ kind: "remote", provider: provider.id, model }),
        label: shortModel(model),
        checked: service.active && model === service.model,
      })),
    });
  }
  return { ready, unavailable };
}

/** The 语音模型 menu: the cloud providers' models, the installed local models (tier name, the
 *  product beside it), the providers that still need setting up, then 管理语音模型…. */
export function speechMenuSections(
  status: EngineStatus,
  models: readonly ModelState[],
  t: TFunction = zhT.t,
  locale: Locale = "zh-CN",
): MenuSection[] {
  const { ready, unavailable } = providerSections(status, "asr", t);
  const local: MenuItem[] = models
    .filter((m) => isRecognitionModel(m) && m.state.kind === "installed")
    .map((m) => ({
      kind: "radio",
      id: choiceId({ kind: "local", id: m.id }),
      label: modelDisplayName(m.id, m.name, locale),
      detail: modelFamilyName(m.id, m.name, locale),
      checked: status.asr_provider === "local" && m.active,
    }));
  const sections = [...ready];
  if (local.length > 0) sections.push({ label: t("switchers.speech.local"), items: local });
  if (unavailable.length > 0)
    sections.push({ label: t("switchers.unavailable"), items: unavailable });
  sections.push({
    items: [
      {
        kind: "action",
        id: choiceId({ kind: "manage", target: "speech" }),
        label: t("switchers.speech.manage"),
      },
    ],
  });
  return sections;
}

/** The AI 润色模型 menu: the LLM providers' models, those still needing setup, 管理 AI 模型…. */
export function polishMenuSections(status: EngineStatus, t: TFunction = zhT.t): MenuSection[] {
  const { ready, unavailable } = providerSections(status, "llm", t);
  const sections = [...ready];
  if (unavailable.length > 0)
    sections.push({ label: t("switchers.unavailable"), items: unavailable });
  sections.push({
    items: [
      {
        kind: "action",
        id: choiceId({ kind: "manage", target: "ai" }),
        label: t("switchers.polish.manage"),
      },
    ],
  });
  return sections;
}

/** What a take records, for the title bar's 麦克风 menu: offered only where the computer's sound
 *  can be recorded (the home card has its own switch). */
export interface SourceChoice {
  current: RecordingSource;
  available: boolean;
}

/** The 麦克风 menu: 系统默认（名称）, every input device by its short name, a chosen device that
 *  is not connected (shown, not choosable), the 录音来源 choice when `source` is given and the
 *  computer's sound can be recorded, then 录音来源设置…. */
export function microphoneMenuSections(
  devices: readonly AudioDevice[],
  chosen: string | null | undefined,
  t: TFunction = zhT.t,
  source?: SourceChoice,
  locale: Locale = "zh-CN",
): MenuSection[] {
  const fallback = devices.find((d) => d.is_default);
  const items: MenuItem[] = [
    {
      kind: "radio",
      id: choiceId({ kind: "microphone", device: null }),
      label:
        fallback === undefined
          ? t("switchers.microphone.default")
          : t("switchers.microphone.defaultNamed", { name: shortMicrophoneName(fallback.name) }),
      checked: chosen === null || chosen === undefined,
    },
    ...devices.map((d) => ({
      kind: "radio" as const,
      id: choiceId({ kind: "microphone", device: d.id }),
      label: shortMicrophoneName(d.name),
      checked: chosen === d.id,
    })),
  ];
  if (chosen !== null && chosen !== undefined && !devices.some((d) => d.id === chosen)) {
    items.push({
      kind: "radio",
      id: choiceId({ kind: "microphone", device: chosen }),
      label: shortMicrophoneName(chosen),
      checked: true,
      disabled: true,
      detail: t("switchers.microphone.missing"),
    });
  }
  const sections: MenuSection[] = [{ label: t("switchers.microphone.devices"), items }];
  if (source?.available === true) {
    sections.push({
      label: t("switchers.microphone.source"),
      items: RECORDING_SOURCES.map((s) => ({
        kind: "radio" as const,
        id: choiceId({ kind: "source", source: s }),
        label: recordingSourceLabel(s, locale),
        checked: source.current === s,
      })),
    });
  }
  return [
    ...sections,
    {
      items: [
        {
          kind: "action",
          id: choiceId({ kind: "manage", target: "microphone" }),
          label: t("switchers.microphone.manage"),
        },
      ],
    },
  ];
}

/** `settings` with `choice` serving `kind`: a provider's model keeps the endpoint the user saved
 *  for it (the built-in service has no settings of its own), a local model switches to on-device
 *  recognition. `undefined` for a choice that is not a model of `kind`. */
export function engineSettingsFor(
  settings: EngineSettings,
  kind: ServiceKind,
  choice: SwitchChoice,
): EngineSettings | undefined {
  if (choice.kind === "local")
    return kind === "asr" ? activateLocalModel(settings, choice.id) : undefined;
  if (choice.kind !== "remote" || choice.provider === "local") return undefined;
  if (choice.provider === "builtin") return withProvider(settings, kind, "builtin");
  const saved = settings.providers?.[choice.provider];
  const baseUrl = (kind === "asr" ? saved?.asr_url : saved?.llm_url) ?? "";
  const next = applyProviderDraft(settings, choice.provider, kind, {
    model: choice.model,
    baseUrl,
  });
  return withProvider(next, kind, choice.provider);
}
