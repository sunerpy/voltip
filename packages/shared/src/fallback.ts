// Fallback models on the pages (docs/dictation.md §3.5, §3.14): the settings with their defaults,
// the list edits the pages make, and what each row shows, from the status the core resolved.
import type {
  EngineIssue,
  EngineSettings,
  EngineStatus,
  FallbackModel,
  FallbackSettings,
  FallbackStatus,
  ModelState,
  ProviderId,
  ServiceKind,
  SwitchReason,
} from "./schema";
import { DEFAULT_LOCALE, type Locale, translate } from "./i18n";
import { MAX_FALLBACK_MODELS } from "./schema";

const OFF: FallbackSettings = { enabled: false, models: [] };
const NOTHING_YET: FallbackStatus = {
  enabled: false,
  in_use: false,
  models: [],
};

/** `settings.asr_fallback` / `llm_fallback`, or off with nothing listed (the core omits that). */
export function fallbackSettingsOf(settings: EngineSettings, kind: ServiceKind): FallbackSettings {
  return (kind === "asr" ? settings.asr_fallback : settings.llm_fallback) ?? OFF;
}

/** `engines.asr_fallback` / `llm_fallback`, or nothing resolved yet. */
export function fallbackStatusOf(engines: EngineStatus, kind: ServiceKind): FallbackStatus {
  return (kind === "asr" ? engines.asr_fallback : engines.llm_fallback) ?? NOTHING_YET;
}

/** `settings` with `kind`'s fallback settings replaced (what `settings_set_engines` sends). */
export function withFallback(
  settings: EngineSettings,
  kind: ServiceKind,
  fallback: FallbackSettings,
): EngineSettings {
  return kind === "asr"
    ? { ...settings, asr_fallback: fallback }
    : { ...settings, llm_fallback: fallback };
}

/** Why a model cannot be added to the list, if it cannot: the list is full, the model is blank
 *  (for the built-in service a blank model is the one its card uses), or it is listed already. */
export function fallbackAddProblem(
  list: readonly FallbackModel[],
  entry: FallbackModel,
): "full" | "blank" | "listed" | undefined {
  if (list.length >= MAX_FALLBACK_MODELS) return "full";
  const model = entry.model.trim();
  if (model.length === 0 && entry.provider !== "builtin") return "blank";
  const listed = list.some((m) => m.provider === entry.provider && m.model.trim() === model);
  return listed ? "listed" : undefined;
}

/** `list` with the entry at `index` moved by `by` places (kept within the list). */
export function moveFallback(
  list: readonly FallbackModel[],
  index: number,
  by: -1 | 1,
): FallbackModel[] {
  const to = index + by;
  if (index < 0 || index >= list.length || to < 0 || to >= list.length) return [...list];
  const next = [...list];
  const [moved] = next.splice(index, 1);
  if (moved !== undefined) next.splice(to, 0, moved);
  return next;
}

/** What a row of the chain shows. */
export type FallbackRowState =
  /** The model requests go to now (only while the chain runs). */
  | "active"
  /** Has quota as far as Voltip knows; asked after the ones above it run out. */
  | "ready"
  /** Ran out of quota; tried again at `retryAtMs`. */
  | "exhausted"
  /** Failed (§3.14); skipped until `retryAtMs`, then asked again. */
  | "cooling"
  /** Cannot run: `issue` says why. */
  | "issue"
  /** The selected model listed again: never asked twice. */
  | "same"
  /** Listed twice. */
  | "duplicate"
  /** A service while recognition runs on this device: a failure never sends the audio away (§3.14). */
  | "not_for_local";

/** The row state of a model skipped until a time, by why. */
function skippedFor(reason: SwitchReason | undefined): FallbackRowState {
  return reason === "outage" ? "cooling" : "exhausted";
}

/** One row of the chain as the pages show it. */
export interface FallbackRowView {
  /** `"selected"` for the selected model, else the index in `FallbackSettings.models`. */
  index: number | "selected";
  provider: ProviderId;
  model: string;
  state: FallbackRowState;
  issue?: EngineIssue;
  retryAtMs?: number;
}

/** The chain of `kind`: the selected model first, then every fallback entry in order. While the
 *  chain runs, the first model with quota left is `active`; `active` is undefined when it does not
 *  run, or when every model ran out (the core then asks the selected one again). */
export function fallbackRows(
  engines: EngineStatus,
  kind: ServiceKind,
): { rows: FallbackRowView[]; active: FallbackRowView | undefined } {
  const status = fallbackStatusOf(engines, kind);
  const provider = kind === "asr" ? engines.asr_provider : engines.llm_provider;
  const model = kind === "asr" ? engines.asr_model : engines.refine_model;
  const rows: FallbackRowView[] = [];
  if (provider !== undefined) {
    const retry = status.in_use ? status.selected_retry_at_ms : undefined;
    rows.push({
      index: "selected",
      provider,
      model,
      state: retry === undefined ? "ready" : skippedFor(status.selected_retry_reason),
      ...(retry === undefined ? {} : { retryAtMs: retry }),
    });
  }
  status.models.forEach((m, index) => {
    const state: FallbackRowState =
      m.issue !== undefined
        ? "issue"
        : m.skip === "same_as_selected"
          ? "same"
          : m.skip === "duplicate"
            ? "duplicate"
            : m.skip === "not_for_local"
              ? "not_for_local"
              : m.retry_at_ms !== undefined
                ? skippedFor(m.retry_reason)
                : "ready";
    rows.push({
      index,
      provider: m.provider,
      model: m.model,
      state,
      ...(m.issue === undefined ? {} : { issue: m.issue }),
      ...(m.retry_at_ms === undefined ? {} : { retryAtMs: m.retry_at_ms }),
    });
  });
  const active = status.in_use ? rows.find((r) => r.state === "ready") : undefined;
  if (active !== undefined) active.state = "active";
  return { rows, active };
}

/** The models of this device a recognition chain may list (§3.14): the library's installed models
 *  that recognise whole takes (`offline`; not the streaming one, which only previews). */
export function localFallbackModels(models: readonly ModelState[]): ModelState[] {
  return models.filter((m) => m.capabilities.includes("offline") && m.state.kind === "installed");
}

/** Whether `kind`'s chain moves on for failures too (服务出错时也改用候补, §3.14). */
export function fallbackOnOutage(settings: EngineSettings, kind: ServiceKind): boolean {
  return fallbackSettingsOf(settings, kind).on_outage === true;
}

/** The providers `kind`'s chain may send the data to besides the selected one (§3.5, §3.14): those
 *  of the fallback models that can run while it runs. A model of this computer sends nothing. */
export function fallbackTargets(engines: EngineStatus, kind: ServiceKind): ProviderId[] {
  if (!fallbackStatusOf(engines, kind).in_use) return [];
  const selected = kind === "asr" ? engines.asr_provider : engines.llm_provider;
  const sending: ReadonlySet<FallbackRowState> = new Set([
    "ready",
    "active",
    "exhausted",
    "cooling",
  ]);
  return [
    ...new Set(
      fallbackRows(engines, kind)
        .rows.filter((r) => r.index !== "selected" && sending.has(r.state))
        .map((r) => r.provider)
        .filter((p) => p !== selected && p !== "local"),
    ),
  ];
}

/** Whose model wrote a history entry's text, and why a fallback model did (§3.14): 「阿里云百炼」,
 *  「阿里云百炼 · 候补模型 · 所选模型出错」. */
export function servedByText(
  provider: ProviderId,
  fallback: SwitchReason | undefined,
  locale: Locale = DEFAULT_LOCALE,
): string {
  const name = translate(locale, `engines.provider.${provider}`);
  return fallback === undefined
    ? name
    : `${name} · ${translate(locale, `fallbackOutage.history.reason.${fallback}`)}`;
}
