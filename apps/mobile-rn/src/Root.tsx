// The app's entry (index.ts): start the Rust shell through the native module, then hand the
// screens a `TauriBackend` over the React Native transport (docs/mobile-rn.md §2).
import { TauriBackend } from "@voltip/shared";
import { useEffect, useState } from "react";
import {
  AppState,
  Linking,
  type Permission,
  PermissionsAndroid,
  Platform,
  Text,
  View,
} from "react-native";

import { type VoltipNativeModule, loadVoltipNative } from "../modules/voltip-native/src";
import { App } from "./App";
import { type BubbleControl, parseBubbleStatus } from "./backend/bubble";
import { createTransport } from "./backend/transport";

/** Ask Android for the microphone (the system dialog) unless it is granted already. */
async function askMicrophone(): Promise<boolean> {
  const permission = PermissionsAndroid.PERMISSIONS.RECORD_AUDIO;
  if (await PermissionsAndroid.check(permission)) return true;
  return (await PermissionsAndroid.request(permission)) === PermissionsAndroid.RESULTS.GRANTED;
}

/** Ask for `permission` from 悬浮球: the system dialog, or `settings` once it was refused for good
 *  (Android shows no dialog then). */
async function askFromSettings(
  permission: Permission,
  settings: () => Promise<void> = () => Linking.openSettings(),
): Promise<boolean> {
  if (await PermissionsAndroid.check(permission)) return true;
  const result = await PermissionsAndroid.request(permission);
  if (result === PermissionsAndroid.RESULTS.NEVER_ASK_AGAIN) {
    await settings();
    return false;
  }
  return result === PermissionsAndroid.RESULTS.GRANTED;
}

/** The floating bubble through the native module (`BubbleService`, docs/mobile-rn.md §11). */
function bubbleControl(native: VoltipNativeModule): BubbleControl {
  return {
    status: async () => parseBubbleStatus(await native.bubbleStatus()),
    configure: async (config) =>
      parseBubbleStatus(await native.bubbleConfigure(JSON.stringify(config))),
    requestMicrophone: () => askFromSettings(PermissionsAndroid.PERMISSIONS.RECORD_AUDIO),
    // Android 13 brought the notification permission; before it, notifications switched off are
    // switched on again in the app's notification settings.
    requestNotifications: () =>
      Platform.OS === "android" && Platform.Version >= 33
        ? askFromSettings(PermissionsAndroid.PERMISSIONS.POST_NOTIFICATIONS, () =>
            native.bubbleOpenNotificationSettings(),
          )
        : native.bubbleOpenNotificationSettings().then(() => false),
    openOverlaySettings: () => native.bubbleOpenOverlaySettings(),
    openAccessibilitySettings: () => native.bubbleOpenAccessibilitySettings(),
    openAppSettings: () => native.bubbleOpenAppSettings(),
  };
}

type Boot =
  | { phase: "starting" }
  | {
      phase: "ready";
      backend: TauriBackend;
      bubble: BubbleControl;
      navigationBar: (lightButtons: boolean) => void;
    }
  | { phase: "failed"; reason: string };

export function Root() {
  const [boot, setBoot] = useState<Boot>({ phase: "starting" });
  const [accent, setAccent] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    const native = loadVoltipNative();
    // The wallpaper's colour, again whenever the app comes back to the front (it may have changed).
    const readAccent = () => {
      setAccent(native.systemAccent());
    };
    readAccent();
    const foreground = AppState.addEventListener("change", (next) => {
      if (next === "active") readAccent();
    });
    native
      .start()
      .then((reason) => {
        if (!alive) return;
        if (reason !== null) {
          setBoot({ phase: "failed", reason });
          return;
        }
        const transport = createTransport(native, {
          askMicrophone,
          warn: (message, detail) => {
            console.warn(message, detail);
          },
        });
        setBoot({
          phase: "ready",
          backend: new TauriBackend(transport),
          bubble: bubbleControl(native),
          navigationBar: (light) => {
            native.setNavigationBarButtons(light).catch(() => undefined);
          },
        });
      })
      .catch((e: unknown) => {
        if (alive) setBoot({ phase: "failed", reason: e instanceof Error ? e.message : String(e) });
      });
    return () => {
      alive = false;
      foreground.remove();
    };
  }, []);
  if (boot.phase === "ready")
    return (
      <App
        backend={boot.backend}
        accent={accent}
        bubble={boot.bubble}
        navigationBar={boot.navigationBar}
      />
    );
  // Before the translator exists (the locale is a core setting): the splash shows no text, a failure
  // the shell's own reason.
  return (
    <View style={{ flex: 1, alignItems: "center", justifyContent: "center", padding: 24 }}>
      {boot.phase === "failed" && <Text selectable>{boot.reason}</Text>}
    </View>
  );
}
