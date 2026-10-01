import { activationChip, isBuiltinPreset, presetLabel } from "@voltip/shared";
import {
  Button,
  Card,
  Chip,
  Eyebrow,
  Icon,
  type IconName,
  Keycaps,
  Lamp,
  LampText,
  Logo,
  Panel,
  Readout,
  Segmented,
  TitleBar,
  cx,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import type { ReactNode } from "react";
import { PresetMenu } from "../features/presets/PresetMenu";
import {
  MicrophoneMenu,
  PolishMenu,
  PolishModelMenu,
  ReadoutTrigger,
  SpeechModelMenu,
} from "../features/switchers/SwitcherMenus";
import { engineReadout } from "../shell/page-meta";

/** The window controls of a drawn title bar: present (so they are drawn), inert. */
const INERT_CONTROLS = {
  minimize: () => undefined,
  toggleMaximize: () => undefined,
  close: () => undefined,
};

/** A window-sized box: the sheet draws the bars at the widths the design is judged at. `height`
 *  leaves room below for a menu that opens. */
function Frame({
  title,
  width,
  height,
  children,
  testId,
}: {
  title: string;
  width: number;
  height?: number;
  children: ReactNode;
  testId: string;
}) {
  return (
    <section className="flex flex-col gap-1.5" data-testid={testId}>
      <span className="mono text-[11px] text-fg-subtle">{title}</span>
      <div className="overflow-x-auto pb-1">
        <div
          className="rounded-10 bg-canvas hairline"
          style={{ width, minHeight: height }}
          data-testid={`${testId}-frame`}>
          {children}
        </div>
      </div>
    </section>
  );
}

/** The 润色 switch as the title bar draws it today (wand + label + lamp), without its menu. */
function PolishSwitch() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const on = state.engines.refine_enabled;
  return (
    <button
      type="button"
      aria-label={t("shell.polish.aria")}
      aria-pressed={on}
      onClick={() => {
        void backend.invoke("settings_set_engines", {
          engines: { ...state.settings.engines, refine_enabled: !on },
        });
      }}
      className="inline-flex h-7 items-center gap-1.5 rounded-6 px-1.5 text-fg-muted transition-colors hover:bg-inset hover:text-fg">
      <Icon name="wand" size={16} className={on ? "text-accent-text" : "text-fg-subtle"} />
      <span className={cx("text-[12px] whitespace-nowrap", on ? "text-fg" : "text-fg-muted")}>
        {t("shell.polish.label")}
      </span>
      <Lamp tone={on ? "ok" : "idle"} size={6} />
    </button>
  );
}

const PRESET_TRIGGER =
  "inline-flex h-7 items-center gap-1 rounded-6 px-1.5 text-[12px] whitespace-nowrap text-fg transition-colors hover:bg-inset";

type Open = "speech" | "microphone" | "polish" | undefined;

/** The title bar of the proposal: the readout's two menus, then 润色 with option A (preset and
 *  model, two menus) or B (one menu for both). `open` draws one menu opened. */
function ProposedTitleBar({ option, open }: { option: "A" | "B"; open?: Open }) {
  const { t, locale } = useI18n();
  const state = useUiState();
  const preset = state.settings.engines.refine_preset;
  return (
    <div className="flex h-10">
      {/* The sidebar's brand row, which continues the strip on the left. */}
      <div className="flex w-[224px] shrink-0 items-center gap-2 border-r border-b border-border bg-nav px-4">
        <Logo size={20} />
        <span className="text-[14px] font-semibold text-fg">Voltip</span>
      </div>
      <div className="min-w-0 flex-1">
        <TitleBar
          title={t("page.title.home")}
          platform="windows"
          controls={INERT_CONTROLS}
          onSearch={() => undefined}
          readout={
            <>
              <SpeechModelMenu defaultOpen={open === "speech"} data-testid="design-bar-speech" />
              <span aria-hidden>·</span>
              <MicrophoneMenu
                withSource
                defaultOpen={open === "microphone"}
                data-testid="design-bar-mic"
              />
            </>
          }
          right={
            <>
              <PolishSwitch />
              {option === "A" ? (
                <>
                  <PresetMenu
                    align="end"
                    triggerClassName={PRESET_TRIGGER}
                    trigger={
                      <>
                        <span {...(isBuiltinPreset(preset) ? {} : { "data-user-text": "" })}>
                          {presetLabel(preset, state.presets, locale)}
                        </span>
                        <Icon name="chevronDown" size={12} className="text-fg-subtle" />
                      </>
                    }
                  />
                  <PolishModelMenu
                    align="end"
                    defaultOpen={open === "polish"}
                    triggerClassName="inline-flex h-7 max-w-[140px] min-w-0 items-center gap-1 rounded-6 px-1.5 mono text-[11px] text-fg-muted transition-colors hover:bg-inset hover:text-fg"
                    data-testid="design-bar-polish"
                  />
                </>
              ) : (
                <PolishMenu
                  align="end"
                  defaultOpen={open === "polish"}
                  triggerClassName="inline-flex h-7 max-w-[220px] min-w-0 items-center gap-1 rounded-6 px-1.5 text-[12px] text-fg transition-colors hover:bg-inset"
                  data-testid="design-bar-polish-b"
                />
              )}
            </>
          }
        />
      </div>
    </div>
  );
}

/** The home page's ready bar with the 语音模型 chip turned into a menu (the preset menu as today). */
function ProposedReadyBar() {
  const i18n = useI18n();
  const { t, locale } = i18n;
  const state = useUiState();
  const engines = state.engines;
  const preset = state.settings.engines.refine_preset;
  return (
    <Card padding="none" className="flex min-h-[52px] flex-wrap items-center gap-3 px-3.5 py-2">
      <Lamp tone="ok" size={10} />
      <div className="flex min-w-[10rem] flex-1 items-baseline gap-2">
        <span className="shrink-0 text-[15px] font-medium text-fg">{t("home.status.ready")}</span>
        <span className="truncate text-[13px] text-fg-muted">{t("home.status.readyDetail")}</span>
      </div>
      <Keycaps keys={state.settings.hotkey} />
      <Chip>{activationChip(state.settings.activation, locale)}</Chip>
      <SpeechModelMenu
        defaultOpen
        triggerClassName="inline-flex h-7 max-w-[260px] min-w-0 items-center gap-1.5 rounded-6 bg-surface px-2.5 text-[12px] text-fg hairline hover:border-fg-subtle"
        trigger={
          // The chip's words as today (「内置服务 · Qwen3-ASR-1.7B」 / 「本机 · 均衡」), now a menu.
          <ReadoutTrigger
            value={
              engines.asr_provider === "local"
                ? t("home.chip.local", { model: engineReadout(engines, i18n).value })
                : t("home.chip.provider", {
                    provider: t(`engines.provider.${engines.asr_provider}`),
                    model: engineReadout(engines, i18n).value,
                  })
            }
          />
        }
        data-testid="design-home-speech"
      />
      <PresetMenu
        align="end"
        triggerClassName={`inline-flex h-7 items-center gap-1.5 rounded-6 bg-surface px-2.5 text-[12px] whitespace-nowrap hairline hover:border-fg-subtle ${engines.refine_enabled ? "text-fg" : "text-fg-muted"}`}
        trigger={
          <>
            <Icon
              name="wand"
              size={14}
              className={engines.refine_enabled ? "text-accent-text" : "text-fg-subtle"}
            />
            <span {...(isBuiltinPreset(preset) ? {} : { "data-user-text": "" })}>
              {presetLabel(preset, state.presets, locale)}
            </span>
            <Icon name="chevronDown" size={12} className="text-fg-subtle" />
          </>
        }
      />
      <Button variant="primary" icon="mic">
        {t("home.button.start")}
      </Button>
    </Card>
  );
}

/** The home page's recording source card: the device name is the menu; 选择设备 goes. */
function ProposedMicCard() {
  const { t } = useI18n();
  return (
    <Panel
      eyebrow={t("home.mic.eyebrow")}
      right={
        <LampText tone="idle" mono>
          {t("home.mic.idle")}
        </LampText>
      }
      className="min-h-[144px]">
      <MicrophoneMenu
        defaultOpen
        triggerClassName="inline-flex items-center gap-1.5 rounded-6 px-1 -mx-1 text-[13px] font-medium text-fg transition-colors hover:bg-inset"
        data-testid="design-card-mic"
      />
      <div className="mono mt-0.5 text-[11px] text-fg-muted">
        48 kHz · {t("settings.microphone.mono")} · {t("home.mic.systemDefault")}
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2">
        <Button size="sm" variant="primary" icon="mic">
          {t("home.mic.test")}
        </Button>
        <span className="min-w-0 flex-1 text-[11px] text-fg-muted">{t("home.mic.idleHint")}</span>
      </div>
    </Panel>
  );
}

/** The home page's speech model card: the model and 润色模型 are menus; 配置语音模型 stays. */
function ProposedEngineCard() {
  const i18n = useI18n();
  const { t, locale } = i18n;
  const state = useUiState();
  const engines = state.engines;
  const preset = state.settings.engines.refine_preset;
  return (
    <Panel
      eyebrow={t("home.engine.eyebrow")}
      right={
        <Button size="sm" variant="ghost" icon="key">
          {t("home.engine.configure")}
        </Button>
      }
      className="min-h-[144px]">
      <SpeechModelMenu
        triggerClassName="inline-flex items-center gap-1.5 rounded-6 px-1 -mx-1 text-[13px] font-medium text-fg transition-colors hover:bg-inset"
        trigger={<ReadoutTrigger value={engineReadout(engines, i18n).value} />}
        data-testid="design-engine-speech"
      />
      <div className="mono mt-0.5 text-[11px] text-fg-muted">
        {t("home.engine.detail", {
          provider: t(`engines.provider.${engines.asr_provider}`),
          language: engines.language ?? t("home.engine.auto"),
        })}
      </div>
      <div className="mt-3 grid grid-cols-3 gap-3">
        <Readout
          label={t("home.engine.refineModel")}
          value={
            <PolishModelMenu
              defaultOpen
              triggerClassName="inline-flex max-w-[200px] items-center gap-1 rounded-6 px-1 -mx-1 mono text-fg transition-colors hover:bg-inset"
              data-testid="design-engine-polish"
            />
          }
          size="sm"
        />
        <Readout
          label={t("home.engine.preset")}
          value={
            <span {...(isBuiltinPreset(preset) ? {} : { "data-user-text": "" })}>
              {presetLabel(preset, state.presets, locale)}
            </span>
          }
          size="sm"
        />
        <Readout label={t("home.engine.inject")} value={t("home.engine.injectPaste")} size="sm" />
      </div>
    </Panel>
  );
}

function TrayLine({ icon, text }: { icon: IconName; text: string }) {
  return (
    <li className="flex items-start gap-2 text-[13px] text-fg">
      <Icon name={icon} size={16} className="mt-0.5 shrink-0 text-fg-muted" />
      <span>{text}</span>
    </li>
  );
}

/** `/design/switchers` (`pnpm dev` only): the proposal of 2026-09-30 for switching the speech
 *  model, the AI polish model and the microphone in place, drawn with the real components on the
 *  preview data; every menu works. The real title bar and home page are untouched until the
 *  design is confirmed. */
export default function SwitchersSheet() {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const state = useUiState();
  const dark = state.settings.theme === "dark";
  return (
    <div
      className="mx-auto flex w-full max-w-[1520px] flex-col gap-6 p-6"
      data-testid="page-design-switchers">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div className="flex max-w-[760px] flex-col gap-1">
          <Eyebrow>{t("design.switchers.heading")}</Eyebrow>
          <p className="text-[13px] text-fg-muted">{t("design.switchers.intro")}</p>
        </div>
        <div className="flex items-center gap-3">
          <Segmented
            size="sm"
            label={t("design.switchers.theme")}
            value={dark ? "dark" : "light"}
            onChange={(theme) => {
              void backend.invoke("settings_set_theme", { theme, followSystem: false });
            }}
            options={[
              { value: "light", label: t("design.switchers.light") },
              { value: "dark", label: t("design.switchers.dark") },
            ]}
          />
          <Segmented
            size="sm"
            label={t("design.switchers.language")}
            value={locale}
            onChange={(next) => {
              void backend.invoke("settings_set_locale", {
                locale: next === "en" ? "en" : "zh-cn",
              });
            }}
            options={[
              { value: "zh-CN", label: "中文" },
              { value: "en", label: "English" },
            ]}
          />
        </div>
      </div>

      <Frame title={t("design.switchers.titleBarA")} width={1440} testId="design-bar-a">
        <ProposedTitleBar option="A" />
      </Frame>
      <Frame title={t("design.switchers.titleBarB")} width={1440} testId="design-bar-b">
        <ProposedTitleBar option="B" />
      </Frame>
      <Frame title={t("design.switchers.narrow")} width={960} testId="design-bar-narrow">
        <ProposedTitleBar option="A" />
      </Frame>

      <Eyebrow>{t("design.switchers.menus")}</Eyebrow>
      <Frame
        title={t("switchers.speech.label")}
        width={1440}
        height={330}
        testId="design-menu-speech">
        <ProposedTitleBar option="A" open="speech" />
      </Frame>
      <Frame
        title={t("switchers.microphone.label")}
        width={1440}
        height={300}
        testId="design-menu-mic">
        <ProposedTitleBar option="A" open="microphone" />
      </Frame>
      <Frame
        title={t("switchers.polish.label")}
        width={1440}
        height={420}
        testId="design-menu-polish">
        <ProposedTitleBar option="A" open="polish" />
      </Frame>
      <Frame
        title={t("design.switchers.titleBarB")}
        width={1440}
        height={560}
        testId="design-menu-polish-b">
        <ProposedTitleBar option="B" open="polish" />
      </Frame>

      <Frame
        title={t("design.switchers.homeReady")}
        width={1152}
        height={360}
        testId="design-home-ready">
        <div className="p-6">
          <ProposedReadyBar />
        </div>
      </Frame>
      <div className="flex flex-wrap gap-6">
        <Frame
          title={t("design.switchers.homeMic")}
          width={600}
          height={360}
          testId="design-home-mic">
          <div className="p-6">
            <ProposedMicCard />
          </div>
        </Frame>
        <Frame
          title={t("design.switchers.homeEngine")}
          width={600}
          height={460}
          testId="design-home-engine">
          <div className="p-6">
            <ProposedEngineCard />
          </div>
        </Frame>
      </div>

      <Frame title={t("design.switchers.tray")} width={760} testId="design-tray">
        <ul className="flex flex-col gap-2 p-5">
          <TrayLine icon="monitor" text={t("design.switchers.trayWindows")} />
          <TrayLine icon="monitor" text={t("design.switchers.trayMac")} />
          <TrayLine icon="terminal" text={t("design.switchers.trayLinux")} />
        </ul>
      </Frame>
    </div>
  );
}
