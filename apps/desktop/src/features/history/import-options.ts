import type {
  EngineStatus,
  Locale,
  ModelState,
  PolishModel,
  RecognitionChoice,
  ServiceKind,
  TFunction,
} from "@voltip/shared";
import type { MenuSection, SelectOption } from "@voltip/ui";
import { parseChoice, polishMenuSections, speechMenuSections } from "../switchers/switchers";

/** The models an import may use, as the import dialog offers them (docs/dictation.md §31): the
 *  rows of the 语音模型 and AI 润色模型 menus that are installed local models, cloud providers with
 *  the user's own key, or Ollama and the custom endpoint as the user set them up, key or not
 *  (§31.4, user decision D20); never the built-in service (the core refuses it too). Each list
 *  starts on the model in use, else on its first row; `undefined` when it has none. */
export interface ImportOptions {
  speech: SelectOption<string>[];
  polish: SelectOption<string>[];
  speechDefault: string | undefined;
  polishDefault: string | undefined;
}

/** The choosable rows of `sections` that an import may use (a local model, or a provider of the
 *  user's own for `kind`), each named after its section (the provider, or 本地模型) so that the
 *  closed select says whose model it is, and the row that is checked. */
function allowedRows(
  sections: readonly MenuSection[],
  status: EngineStatus,
  kind: ServiceKind,
): {
  options: SelectOption<string>[];
  checked: string | undefined;
} {
  const options: SelectOption<string>[] = [];
  let checked: string | undefined;
  for (const section of sections) {
    for (const item of section.items) {
      if (item.kind !== "radio" || item.disabled === true) continue;
      const choice = parseChoice(item.id);
      if (choice?.kind === "remote" && !usersOwn(status, choice.provider, kind)) continue;
      const name = item.detail === undefined ? item.label : `${item.label} · ${item.detail}`;
      options.push({
        value: item.id,
        label: section.label === undefined ? name : `${section.label} · ${name}`,
      });
      if (item.checked) checked ??= item.id;
    }
  }
  return { options, checked };
}

/** `provider`'s `kind` service is the user's own, as the core checks it (`import_engines`): one
 *  they set up that takes no key or may go without one (Ollama, the custom endpoint), or one with
 *  the key they entered. The built-in service is never theirs: its key is the build's. */
function usersOwn(status: EngineStatus, provider: string, kind: ServiceKind): boolean {
  const entry = status.providers.find((p) => p.id === provider);
  if (entry?.key === "none" || entry?.key === "optional") return true;
  const key = entry?.[kind]?.key;
  return entry?.key === "required" && key?.set === true && key.source === "user";
}

export function importOptions(
  status: EngineStatus,
  models: readonly ModelState[],
  t: TFunction,
  locale: Locale,
): ImportOptions {
  const speech = allowedRows(speechMenuSections(status, models, t, locale), status, "asr");
  const polish = allowedRows(polishMenuSections(status, t), status, "llm");
  return {
    speech: speech.options,
    polish: polish.options,
    speechDefault: speech.checked ?? speech.options[0]?.value,
    polishDefault: polish.checked ?? polish.options[0]?.value,
  };
}

/** What `import_add` sends for a row of the speech list. */
export function recognitionOf(id: string): RecognitionChoice | undefined {
  const choice = parseChoice(id);
  return choice?.kind === "remote" || choice?.kind === "local" ? choice : undefined;
}

/** What `import_add` sends for a row of the polish list. */
export function polishOf(id: string): PolishModel | undefined {
  const choice = parseChoice(id);
  return choice?.kind === "remote" ? { provider: choice.provider, model: choice.model } : undefined;
}
