import {
  type Activation,
  type DeviceView,
  type EngineStatus,
  type HotkeyStatus,
  type RelayStatus,
  type ThemeId,
  type Translator,
  type UiState,
  activationShortcut,
  engineReady,
  enginesReported,
  modelDisplayName,
  platformLabel,
  relayLabel,
  themeName,
  zhT,
} from "@voltip/shared";
import type { ToolbarReadout } from "@voltip/ui";
import { type BackgroundRoute, HOME_ROUTE, type Route, type SettingsSection } from "../app/router";

export interface PageMeta {
  title: string;
  readouts: ToolbarReadout[];
  shortcuts: readonly (readonly [string, string])[];
}

/** `Qwen/Qwen3-ASR-1.7B` → `Qwen3-ASR-1.7B`: the model id without its vendor prefix. */
export function shortModel(model: string): string {
  const tail = model.split("/").at(-1) ?? model;
  return tail.length > 0 ? tail : model;
}

/** Title-bar readout for the resolved recognition (`state.engines`), or a pending marker before
 *  the core reported. The model with a lamp that follows `asr_ready`; on-device a `本机` tag; the
 *  tooltip names the provider (never the built-in service's host) and why it cannot run. */
export function engineReadout(engines: EngineStatus, i18n: Translator = zhT): ToolbarReadout {
  const { t, locale } = i18n;
  if (!enginesReported(engines))
    return {
      label: t("shell.readout.engine"),
      value: t("shell.readout.waitingCore"),
      lamp: "idle",
    };
  const ready = engineReady(engines);
  const provider = t(`engines.provider.${engines.asr_provider}`);
  const state = ready
    ? t("engines.ready")
    : t(`engines.issue.${engines.asr_issue ?? "unavailable"}`);
  if (engines.asr_provider === "local") {
    // The core's display name (zh) or the dictionary's name by id under en.
    const model = modelDisplayName(engines.local_model ?? "", engines.asr_model, locale);
    return {
      label: t("shell.readout.engine"),
      value: model,
      lamp: ready ? "ok" : "danger",
      badge: t("shell.readout.local"),
      title: t("shell.readout.providerTitle", { provider, model, state }),
    };
  }
  return {
    label: t("shell.readout.engine"),
    value: shortModel(engines.asr_model),
    lamp: ready ? "ok" : "danger",
    title: t("shell.readout.providerTitle", { provider, model: engines.asr_model, state }),
  };
}

/** The engines group's readouts: the resolved ASR, the polish state and the injection mode. */
function engineReadouts(engines: EngineStatus, i18n: Translator): ToolbarReadout[] {
  const { t } = i18n;
  return [
    engineReadout(engines, i18n),
    {
      label: t("page.readout.polish"),
      value: engines.refine_enabled
        ? t("page.readout.polishOn", { model: shortModel(engines.refine_model) })
        : t("page.readout.polishOff"),
      lamp: engines.refine_enabled ? "ok" : "idle",
    },
    {
      label: t("page.readout.inject"),
      value:
        engines.inject === "paste"
          ? t("page.readout.injectPaste")
          : t("page.readout.injectClipboard"),
    },
  ];
}

/** Title-bar readout for the microphone the native meter is on (`microphoneReadoutValue`). */
export function microphoneReadout(value: string, i18n: Translator = zhT): ToolbarReadout {
  return { label: i18n.t("shell.readout.microphone"), value };
}

/** Footer line for the dictation chord: the saved hotkey, never a fixture, captioned by the
 *  activation mode (docs/dictation.md §13): 按住听写 / 按一下听写 / 按住或按一下听写. */
export function hotkeyShortcut(
  hotkey: string,
  activation: Activation = "hold",
  i18n: Translator = zhT,
): readonly [string, string] {
  return [hotkey.replaceAll("+", " "), activationShortcut(activation, i18n.locale)];
}

/** Toolbar readout for the hotkey pane: the shell's backend name, or the registration failure. */
export function hotkeyBackendReadout(status: HotkeyStatus, i18n: Translator = zhT): string {
  if (status.error) return i18n.t("page.hotkeyBackend.failed");
  if (status.backend.length === 0) return i18n.t("page.hotkeyBackend.notReported");
  // "global-shortcut · Windows · RegisterHotKey" → keep the platform-specific tail.
  const parts = status.backend.split(" · ");
  return parts.length > 1 ? parts.slice(1).join(" · ") : status.backend;
}

function devicesReadouts(
  devices: readonly DeviceView[],
  relay: RelayStatus,
  i18n: Translator,
): ToolbarReadout[] {
  const { t, locale } = i18n;
  const online = devices.filter((d) => d.connection.state === "online").length;
  const link = relayLabel(relay, locale);
  return [
    {
      label: t("page.readout.phones"),
      value: t("page.readout.phonesValue", { paired: devices.length, online }),
      lamp: online > 0 ? "ok" : "idle",
    },
    {
      label: t("page.readout.relay"),
      value: link.text,
      lamp: link.tone === "neutral" ? "idle" : link.tone,
    },
  ];
}

/** Mono readouts in the settings dialog header (hotkey, appearance and the engines group); the
 *  title bar keeps describing the page beneath. */
export function settingsReadouts(
  section: SettingsSection,
  state: UiState,
  appearance: { resolvedTheme: ThemeId; density: string; fontSizePx: number },
  i18n: Translator = zhT,
): ToolbarReadout[] {
  const { t, locale } = i18n;
  switch (section) {
    case "engine":
      return engineReadouts(state.engines, i18n);
    case "hotkey":
      return [
        {
          label: t("page.settingsReadout.hotkey"),
          value: state.settings.hotkey.replaceAll("+", " "),
        },
        {
          label: t("page.settingsReadout.backend"),
          value: hotkeyBackendReadout(state.hotkey, i18n),
        },
      ];
    case "appearance":
      return [
        {
          label: t("page.settingsReadout.theme"),
          value: `${themeName(appearance.resolvedTheme, locale)} · ${state.settings.follow_system_theme ? t("theme.followSystem") : t("theme.notFollowing")}`,
        },
        {
          label: t("page.settingsReadout.density"),
          value: `${appearance.density === "compact" ? t("page.settingsReadout.compact") : t("page.settingsReadout.default")} · ${appearance.fontSizePx} px`,
        },
      ];
    case "scene": {
      const onOff = (on: boolean) =>
        on ? t("page.settingsReadout.on") : t("page.settingsReadout.off");
      const sharing = state.settings.context_sharing;
      return [
        {
          label: t("page.settingsReadout.scenes"),
          value: t("page.settingsReadout.scenesValue", {
            n: state.scenes.length,
            enabled: state.scenes.filter((s) => s.enabled).length,
          }),
        },
        {
          label: t("page.settingsReadout.context"),
          value: t("page.settingsReadout.contextValue", {
            app: onOff(sharing.app_name),
            title: onOff(sharing.window_title),
          }),
        },
      ];
    }
    default:
      return [];
  }
}

const ONBOARDING_STEP_KEYS = ["permissions", "hotkey", "engine", "trial"] as const;

export interface PageMetaExtras {
  /** The microphone the native meter is on (`microphoneReadoutValue`), never a fixture. */
  microphone: string;
}

/** Title, human-readable readouts and footer shortcuts for each route. The settings dialog floats
 *  over `background`, so a settings route describes that page, not itself. The title bar itself
 *  shows only the compact engine · microphone pair (`engineReadout` + `microphoneReadout`); the
 *  page readouts here feed the settings header and tests. */
export function pageMeta(
  route: Route,
  state: UiState,
  extras: PageMetaExtras,
  background: BackgroundRoute = HOME_ROUTE,
  i18n: Translator = zhT,
): PageMeta {
  const { t, locale } = i18n;
  const sc = (key: Parameters<typeof t>[0]) => t(key);
  switch (route.name) {
    case "home":
      return {
        title: t("page.title.home"),
        readouts: [engineReadout(state.engines, i18n), microphoneReadout(extras.microphone, i18n)],
        shortcuts: [
          hotkeyShortcut(state.settings.hotkey, state.settings.activation, i18n),
          ["Ctrl H", sc("page.shortcut.history")],
          ["Ctrl ,", sc("page.shortcut.settings")],
        ],
      };
    case "history":
      return {
        title: t("page.title.history"),
        readouts: [
          {
            label: t("page.readout.history"),
            value: t("page.readout.historyValue", {
              n: state.history.length,
              limit: state.settings.history.keep,
            }),
            lamp: state.history.length > 0 ? "ok" : "idle",
          },
          { label: t("page.readout.storage"), value: "history.json", mono: true },
        ],
        shortcuts: [
          ["Ctrl F", sc("page.shortcut.search")],
          ["Ctrl C", sc("page.shortcut.copy")],
          ["Del", sc("page.shortcut.delete")],
        ],
      };
    case "dictionary": {
      const enabled = state.dictionary.filter((e) => e.enabled).length;
      return {
        title: t("page.title.dictionary"),
        readouts: [
          engineReadout(state.engines, i18n),
          {
            label: t("page.readout.dictionary"),
            value: t("page.readout.enabledValue", { enabled, total: state.dictionary.length }),
            lamp: enabled > 0 ? "ok" : "idle",
          },
          { label: t("page.readout.storage"), value: "dictionary.json", mono: true },
        ],
        shortcuts: [
          ["Ctrl N", sc("page.shortcut.newEntry")],
          ["Enter", sc("page.shortcut.save")],
          ["Esc", sc("page.shortcut.cancel")],
        ],
      };
    }
    case "rules": {
      const enabled = state.rules.filter((r) => r.enabled).length;
      return {
        title: t("page.title.rules"),
        readouts: [
          {
            label: t("page.readout.rules"),
            value: t("page.readout.enabledValue", { enabled, total: state.rules.length }),
            lamp: enabled > 0 ? "ok" : "idle",
          },
          { label: t("page.readout.storage"), value: "rules.json", mono: true },
          engineReadout(state.engines, i18n),
        ],
        shortcuts: [
          ["Ctrl N", sc("page.shortcut.newRule")],
          ["Ctrl ↵", sc("page.shortcut.runDryRun")],
          ["Ctrl S", sc("page.shortcut.save")],
          ["Esc", sc("page.shortcut.cancel")],
        ],
      };
    }
    case "devices":
      return {
        title: t("page.title.devices"),
        readouts: devicesReadouts(state.devices, state.relay, i18n),
        shortcuts: [
          ["Ctrl R", sc("page.shortcut.regenerateQr")],
          ["Ctrl ,", sc("page.shortcut.settings")],
        ],
      };
    case "settings":
      return pageMeta(background, state, extras, HOME_ROUTE, i18n);
    case "onboarding": {
      const stepKey = ONBOARDING_STEP_KEYS[route.step - 1];
      return {
        title: t("page.title.onboarding", { n: route.step }),
        readouts: [
          {
            label: t("page.readout.step"),
            value: t("page.readout.stepValue", {
              n: route.step,
              name: stepKey ? t(`onboarding.steps.${stepKey}`) : "",
            }),
          },
          {
            label: t("page.readout.system"),
            value: state.identity
              ? platformLabel(state.identity.platform, locale)
              : t("common.unknown"),
          },
        ],
        shortcuts: [
          ["Enter", sc("page.shortcut.continue")],
          ["Shift Enter", sc("page.shortcut.previous")],
          ["Esc", sc("page.shortcut.later")],
          ["Ctrl ,", sc("page.shortcut.settings")],
        ],
      };
    }
    case "overlay":
      return {
        title: t("page.title.overlay"),
        readouts: [
          {
            label: t("page.readout.position"),
            value: t(`page.readout.overlay.${state.settings.overlay}`),
            mono: true,
          },
          engineReadout(state.engines, i18n),
        ],
        shortcuts: [
          [state.settings.hotkey.replaceAll("+", " "), sc("page.shortcut.dictate")],
          ["Esc", sc("page.shortcut.cancel")],
          ["Ctrl ,", sc("page.shortcut.settings")],
        ],
      };
    case "notfound":
      return { title: t("page.title.notfound"), readouts: [], shortcuts: [] };
  }
}
