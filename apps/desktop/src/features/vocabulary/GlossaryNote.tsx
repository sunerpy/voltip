import { type EngineStatus, type TFunction } from "@voltip/shared";
import { useI18n, useUiState } from "@voltip/ui";

/** What the recognition service in use does with the glossary (docs/dictation.md §30.6), from
 *  `state.engines.glossary`; nothing while the core cannot tell (no service can run). With no
 *  enabled entry, a glossary goes out only as an enabled built-in scene's terms, in its
 *  applications (`entries` false). */
export function glossaryNote(
  engines: EngineStatus,
  entries: boolean,
  t: TFunction,
): string | undefined {
  switch (engines.glossary) {
    case undefined:
      return undefined;
    case "prompt":
    case "hotwords":
      return t(entries ? "dictionary.glossary.receives" : "dictionary.glossary.scenes");
    case "empty":
      return t("dictionary.glossary.empty");
    case "not_supported":
      return t(
        engines.asr_provider === "local"
          ? "dictionary.glossary.local"
          : "dictionary.glossary.notSupported",
      );
  }
}

/** The sentence, on the dictionary page and under the recognition providers. */
export function GlossaryNote({ className }: { className: string }) {
  const { t } = useI18n();
  const state = useUiState();
  const note = glossaryNote(
    state.engines,
    state.dictionary.some((entry) => entry.enabled),
    t,
  );
  if (note === undefined) return null;
  return (
    <p className={className} data-testid="glossary-note">
      {note}
    </p>
  );
}
