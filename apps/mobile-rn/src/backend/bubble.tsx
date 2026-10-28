// The floating bubble's native side as the screens reach it (docs/mobile-rn.md §11; the Kotlin
// `BubbleService` of modules/voltip-native): its permissions and state, switching it on and off,
// and its options. The app hands in the real one (Root.tsx), tests a fake; without one 设置 shows no
// 悬浮球. The bubble's own texts come from the dictionaries like every other (AGENTS.md): the app
// hands them over with its options, and the service keeps the last ones it was given.
import type { TFunction } from "@voltip/shared";
import { type ReactNode, createContext, useContext, useEffect } from "react";

import { useI18n } from "./i18n";

/** Hold to talk, or tap to start and tap again to finish. */
export type BubbleMode = "hold" | "tap";

/** What the bubble needs from Android, each granted or not. */
export interface BubblePermissions {
  microphone: boolean;
  /** 显示在其他应用上层 (`SYSTEM_ALERT_WINDOW`). */
  overlay: boolean;
  /** Android 13+ asks for it; granted before that. */
  notifications: boolean;
}

export interface BubbleStatus {
  /** The user switched it on: it comes back when the app starts. */
  enabled: boolean;
  /** Its service runs now (the bubble shows). */
  running: boolean;
  permissions: BubblePermissions;
  mode: BubbleMode;
  /** The result shows beside the bubble once copied. */
  showResult: boolean;
  /** The result's card has 不复制 too (off unless asked for, 2026-10-28). */
  showDiscard: boolean;
  pasteEnabled: boolean;
  accessibilityEnabled: boolean;
  autoStowSeconds: 0 | 5 | 10 | 30;
}

/** The texts the bubble and its notification show (`mobile.bubble.native`). */
export type BubbleStrings = Record<BubbleStringKey, string>;

export const BUBBLE_STRING_KEYS = [
  "channel",
  "title",
  "bodyHold",
  "bodyTap",
  "close",
  "label",
  "recordingHold",
  "recordingTap",
  "releaseCancel",
  "cancel",
  "processing",
  "refining",
  "copied",
  "again",
  "discard",
  "noSpeech",
  "failed",
  "quota",
  "holdHint",
  "microphone",
  "expand",
  "stow",
  "releasePaste",
  "pasted",
  "pasteFallback",
] as const;
export type BubbleStringKey = (typeof BUBBLE_STRING_KEYS)[number];

export interface BubbleConfig {
  enabled: boolean;
  mode: BubbleMode;
  showResult: boolean;
  showDiscard: boolean;
  pasteEnabled: boolean;
  autoStowSeconds: 0 | 5 | 10 | 30;
  strings: BubbleStrings;
}

export interface BubbleControl {
  status(): Promise<BubbleStatus>;
  /** Save the options and show or hide the bubble: it shows once enabled with every permission
   *  granted. The answer is the state after. */
  configure(config: BubbleConfig): Promise<BubbleStatus>;
  /** Ask for the microphone (the system dialog, or the app's settings once it was refused for
   *  good): `true` once granted. */
  requestMicrophone(): Promise<boolean>;
  /** Ask for notifications the same way. */
  requestNotifications(): Promise<boolean>;
  /** Open the system's 显示在其他应用上层 page for Voltip. */
  openOverlaySettings(): Promise<void>;
  openAccessibilitySettings(): Promise<void>;
}

/** The bubble's texts in the app's language; `{message}` stays for the bubble to fill in. */
export function bubbleStrings(t: TFunction): BubbleStrings {
  const s = (key: BubbleStringKey) => t(`mobile.bubble.native.${key}`, { message: "{message}" });
  return {
    channel: s("channel"),
    title: s("title"),
    bodyHold: s("bodyHold"),
    bodyTap: s("bodyTap"),
    close: s("close"),
    label: s("label"),
    recordingHold: s("recordingHold"),
    recordingTap: s("recordingTap"),
    releaseCancel: s("releaseCancel"),
    cancel: s("cancel"),
    processing: s("processing"),
    refining: s("refining"),
    copied: s("copied"),
    again: s("again"),
    discard: s("discard"),
    noSpeech: s("noSpeech"),
    failed: s("failed"),
    quota: s("quota"),
    holdHint: s("holdHint"),
    microphone: s("microphone"),
    expand: s("expand"),
    stow: s("stow"),
    releasePaste: s("releasePaste"),
    pasted: s("pasted"),
    pasteFallback: s("pasteFallback"),
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function flag(record: Record<string, unknown>, key: string): boolean {
  const v = record[key];
  if (typeof v !== "boolean") throw new Error(`bubble status: ${key} is not a boolean`);
  return v;
}

/** `BubbleStatus` from the native module's JSON; a malformed answer is an error, not a guess. */
export function parseBubbleStatus(json: string): BubbleStatus {
  const value: unknown = JSON.parse(json);
  const permissions = isRecord(value) ? value["permissions"] : undefined;
  if (!isRecord(value) || !isRecord(permissions)) throw new Error("bubble status: not an object");
  return {
    enabled: flag(value, "enabled"),
    running: flag(value, "running"),
    permissions: {
      microphone: flag(permissions, "microphone"),
      overlay: flag(permissions, "overlay"),
      notifications: flag(permissions, "notifications"),
    },
    mode: value["mode"] === "tap" ? "tap" : "hold",
    showResult: flag(value, "showResult"),
    showDiscard: flag(value, "showDiscard"),
    pasteEnabled: flag(value, "pasteEnabled"),
    accessibilityEnabled: flag(value, "accessibilityEnabled"),
    autoStowSeconds: [0, 5, 10, 30].includes(value["autoStowSeconds"] as number) ? (value["autoStowSeconds"] as 0 | 5 | 10 | 30) : 0,
  };
}

/** How many of the permissions are granted. */
export function grantedCount(permissions: BubblePermissions): number {
  return [permissions.microphone, permissions.overlay, permissions.notifications].filter(Boolean)
    .length;
}

const BubbleContext = createContext<BubbleControl | undefined>(undefined);

export function BubbleProvider({
  control,
  children,
}: {
  control: BubbleControl | undefined;
  children: ReactNode;
}) {
  return <BubbleContext.Provider value={control}>{children}</BubbleContext.Provider>;
}

/** The bubble, or `undefined` where there is none. */
export function useBubble(): BubbleControl | undefined {
  return useContext(BubbleContext);
}

/** Bring a bubble the user switched on back with the app, in the app's language: the service
 *  starts only from the app in front (Android lets no background start use the microphone). */
export function useBubbleSync() {
  const control = useBubble();
  const { t } = useI18n();
  useEffect(() => {
    if (control === undefined) return;
    let alive = true;
    control
      .status()
      .then((status) => {
        if (!alive || !status.enabled) return undefined;
        return control.configure({
          enabled: true,
          mode: status.mode,
          showResult: status.showResult,
          showDiscard: status.showDiscard,
          pasteEnabled: status.pasteEnabled,
          autoStowSeconds: status.autoStowSeconds,
          strings: bubbleStrings(t),
        });
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
    // Again when the language changes (a new `t`): the bubble's texts follow it.
  }, [control, t]);
}
