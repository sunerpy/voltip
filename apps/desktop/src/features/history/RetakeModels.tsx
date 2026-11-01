import type {
  EngineStatus,
  Locale,
  ModelState,
  PolishChoice,
  RecognitionChoice,
  RetakeModels,
  TFunction,
} from "@voltip/shared";
import { Icon, type IconName, Menu, type MenuSection, cx, useI18n, useUiState } from "@voltip/ui";
import { useState } from "react";
import { AI_ROUTE, SPEECH_ROUTE, useRouter } from "../../app/router";
import { engineReadout } from "../../shell/page-meta";
import { usePolishModelName } from "../switchers/SwitcherMenus";
import {
  choiceId,
  parseChoice,
  polishMenuSections,
  speechMenuSections,
} from "../switchers/switchers";

/** The AI 润色 menu's 不润色 row. */
export const POLISH_OFF_ID = "retake:polish-off";
/** The 语音模型 menu's rows for the silence (docs/dictation.md §30.4): as the settings trim it, or kept. */
export const TRIM_ID = "retake:trim";
export const KEEP_SILENCE_ID = "retake:keep-silence";

/** A picker's button: the model's icon and short name, then the menu chevron. */
const PICKER =
  "inline-flex h-7 min-w-0 max-w-[200px] items-center gap-1.5 rounded-control border border-field-edge bg-field px-2 text-[12px] text-fg-muted transition-colors hover:text-fg disabled:cursor-not-allowed disabled:opacity-50";

/** `sections` with no row but `id` checked. */
function checkOnly(sections: readonly MenuSection[], id: string): MenuSection[] {
  return sections.map((section) => ({
    ...section,
    items: section.items.map((item) =>
      item.kind === "radio" ? { ...item, checked: item.id === id } : item,
    ),
  }));
}

/** Whether a row `id` can be chosen in `sections`. */
function offers(sections: readonly MenuSection[], id: string): boolean {
  return sections.some((s) =>
    s.items.some((item) => item.kind === "radio" && item.id === id && item.disabled !== true),
  );
}

/** The 语音模型 menu's section for the silence. */
function isSilence(section: MenuSection): boolean {
  return section.items.some((item) => item.id === TRIM_ID);
}

/** The label of the checked row, if one is. */
function checkedLabel(sections: readonly MenuSection[]): string | undefined {
  for (const section of sections) {
    for (const item of section.items) if (item.kind === "radio" && item.checked) return item.label;
  }
  return undefined;
}

/** The two menus of a 重新识别 and what they send. */
export interface RetakeMenus {
  speech: MenuSection[];
  polish: MenuSection[];
  /** The choices as `audio_retake` sends them. A choice its menu no longer offers (its provider
   *  was set up differently meanwhile) is the settings' again. */
  models: RetakeModels;
}

/** The menus for `chosen` (docs/dictation.md §26.4): the 语音模型 menu's providers and installed
 *  local models, the AI 润色 menu's models with 不润色 first. Nothing chosen checks the settings'
 *  models, as the title bar's menus do; a choice checks itself alone. When the retake runs
 *  on-device and the settings trim the silence (`vadTrim`), the 语音模型 menu ends with 裁剪首尾静音
 *  and 不裁剪静音 (§30.4). */
export function retakeMenus(
  status: EngineStatus,
  models: readonly ModelState[],
  chosen: RetakeModels,
  t: TFunction,
  locale: Locale,
  vadTrim = false,
): RetakeMenus {
  const speechAll = speechMenuSections(status, models, t, locale);
  const asrId = chosen.asr === undefined ? undefined : choiceId(chosen.asr);
  const asr = asrId !== undefined && offers(speechAll, asrId) ? chosen.asr : undefined;
  const picked = asr === undefined || asrId === undefined ? speechAll : checkOnly(speechAll, asrId);
  const onDevice = asr === undefined ? status.asr_provider === "local" : asr.kind === "local";
  const trims = vadTrim && onDevice;
  const keepSilence = trims && chosen.vadTrim === false;
  const speech: MenuSection[] = trims
    ? [
        ...picked,
        {
          label: t("history.recordings.models.silence"),
          items: [
            {
              kind: "radio",
              id: TRIM_ID,
              label: t("history.recordings.models.trim"),
              checked: !keepSilence,
            },
            {
              kind: "radio",
              id: KEEP_SILENCE_ID,
              label: t("history.recordings.models.keepSilence"),
              checked: keepSilence,
            },
          ],
        },
      ]
    : picked;

  const polishAll: MenuSection[] = [
    {
      items: [
        {
          kind: "radio",
          id: POLISH_OFF_ID,
          label: t("history.recordings.models.noPolish"),
          checked: false,
        },
      ],
    },
    ...polishMenuSections(status, t),
  ];
  const refineId =
    chosen.refine === undefined
      ? undefined
      : chosen.refine.kind === "off"
        ? POLISH_OFF_ID
        : choiceId(chosen.refine);
  const refine = refineId !== undefined && offers(polishAll, refineId) ? chosen.refine : undefined;
  const polish =
    refine !== undefined && refineId !== undefined
      ? checkOnly(polishAll, refineId)
      : status.refine_enabled
        ? polishAll
        : checkOnly(polishAll, POLISH_OFF_ID);
  return {
    speech,
    polish,
    models: {
      ...(asr === undefined ? {} : { asr }),
      ...(refine === undefined ? {} : { refine }),
      ...(keepSilence ? { vadTrim: false } : {}),
    },
  };
}

/** The models the next 重新识别 runs on, chosen in place (user request 2026-10-26: on the home
 *  page, without opening an entry). They are for those takes only; the settings stay as they are
 *  (user decision 2026-10-26). 管理… at a menu's end opens the page behind it. */
export function useRetakeModels() {
  const state = useUiState();
  const i18n = useI18n();
  const { t, locale } = i18n;
  const { navigate } = useRouter();
  const [chosen, setChosen] = useState<RetakeModels>({});
  const menus = retakeMenus(
    state.engines,
    state.models,
    chosen,
    t,
    locale,
    state.settings.engines.vad_trim,
  );
  const polishName = usePolishModelName();
  const keepsSilence = menus.models.vadTrim === false;
  const model =
    checkedLabel(menus.speech.filter((s) => !isSilence(s))) ??
    engineReadout(state.engines, i18n).value;
  return {
    ...menus,
    keepsSilence,
    speechName: keepsSilence ? t("history.recordings.models.untrimmed", { name: model }) : model,
    polishName: checkedLabel(menus.polish) ?? polishName,
    chooseSpeech: (id: string) => {
      if (id === KEEP_SILENCE_ID) {
        setChosen((c) => ({ ...c, vadTrim: false }));
        return;
      }
      if (id === TRIM_ID) {
        setChosen(({ vadTrim: _kept, ...c }) => c);
        return;
      }
      const choice = parseChoice(id);
      if (choice?.kind === "manage") {
        navigate(SPEECH_ROUTE);
        return;
      }
      const asr: RecognitionChoice | undefined =
        choice?.kind === "remote" || choice?.kind === "local" ? choice : undefined;
      if (asr !== undefined) setChosen((c) => ({ ...c, asr }));
    },
    choosePolish: (id: string) => {
      if (id === POLISH_OFF_ID) {
        setChosen((c) => ({ ...c, refine: { kind: "off" } }));
        return;
      }
      const choice = parseChoice(id);
      if (choice?.kind === "manage") {
        navigate(AI_ROUTE);
        return;
      }
      const refine: PolishChoice | undefined = choice?.kind === "remote" ? choice : undefined;
      if (refine !== undefined) setChosen((c) => ({ ...c, refine }));
    },
  };
}

export type RetakeModelChoice = ReturnType<typeof useRetakeModels>;

function PickerTrigger({ icon, name }: { icon: IconName; name: string }) {
  return (
    <>
      <Icon name={icon} size={13} className="shrink-0 text-fg-subtle" />
      <span className="truncate">{name}</span>
      <Icon name="chevronDown" size={10} className="shrink-0 opacity-70" />
    </>
  );
}

/** 重新识别使用 [语音模型] [AI 润色]: the two menus of `choice`, side by side. */
export function RetakeModelPickers({
  choice,
  disabled = false,
  className,
}: {
  choice: RetakeModelChoice;
  disabled?: boolean;
  className?: string;
}) {
  const { t } = useI18n();
  const hint = t("history.recordings.models.hint");
  const speech = t("history.recordings.models.speechTrigger", { name: choice.speechName });
  const polish = t("history.recordings.models.polishTrigger", { name: choice.polishName });
  return (
    <div
      className={cx("flex min-w-0 flex-wrap items-center gap-1.5", className)}
      data-testid="retake-models">
      <span className="shrink-0 text-[12px] text-fg-subtle" title={hint}>
        {t("history.recordings.models.using")}
      </span>
      <Menu
        trigger={<PickerTrigger icon="wave" name={choice.speechName} />}
        label={t("history.recordings.models.speech")}
        triggerLabel={speech}
        title={`${speech}\n${hint}`}
        sections={choice.speech}
        onSelect={choice.chooseSpeech}
        disabled={disabled}
        triggerClassName={PICKER}
        data-testid="retake-speech"
      />
      <Menu
        trigger={<PickerTrigger icon="sparkles" name={choice.polishName} />}
        label={t("history.recordings.models.polish")}
        triggerLabel={polish}
        title={`${polish}\n${hint}`}
        sections={choice.polish}
        onSelect={choice.choosePolish}
        disabled={disabled}
        triggerClassName={PICKER}
        data-testid="retake-polish"
      />
    </div>
  );
}
