// The app's entry (index.ts): start the Rust shell through the native module, then hand the
// screens a `TauriBackend` over the React Native transport (docs/mobile-rn.md §2).
import { TauriBackend } from "@voltip/shared";
import { useEffect, useState } from "react";
import { AppState, PermissionsAndroid, Text, View } from "react-native";

import { loadVoltipNative } from "../modules/voltip-native/src";
import { App } from "./App";
import { createTransport } from "./backend/transport";

/** Ask Android for `RECORD_AUDIO` (the system dialog) unless it is granted already. */
async function askMicrophone(): Promise<boolean> {
  const permission = PermissionsAndroid.PERMISSIONS.RECORD_AUDIO;
  if (await PermissionsAndroid.check(permission)) return true;
  return (await PermissionsAndroid.request(permission)) === PermissionsAndroid.RESULTS.GRANTED;
}

type Boot =
  | { phase: "starting" }
  | { phase: "ready"; backend: TauriBackend }
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
        setBoot({ phase: "ready", backend: new TauriBackend(transport) });
      })
      .catch((e: unknown) => {
        if (alive) setBoot({ phase: "failed", reason: e instanceof Error ? e.message : String(e) });
      });
    return () => {
      alive = false;
      foreground.remove();
    };
  }, []);
  if (boot.phase === "ready") return <App backend={boot.backend} accent={accent} />;
  // Before the translator exists (the locale is a core setting): the splash shows no text, a failure
  // the shell's own reason.
  return (
    <View style={{ flex: 1, alignItems: "center", justifyContent: "center", padding: 24 }}>
      {boot.phase === "failed" && <Text selectable>{boot.reason}</Text>}
    </View>
  );
}
