import { type AudioDevice, type LevelFrame, zhT } from "@voltip/shared";
import { useBackend, useT } from "@voltip/ui";
import { useEffect, useState } from "react";
import { publishMicrophone } from "./mic-store";

export interface AudioMeterState {
  /** Microphones the native backend enumerated, default first; `undefined` until the query resolved. */
  devices: AudioDevice[] | undefined;
  /** The device the meter streams from: `deviceId`'s while it is connected, the default input
   *  otherwise (the shell falls back the same way, for the meter and for a take). */
  device: AudioDevice | undefined;
  /** `deviceId` was given but is not among the enumerated devices (unplugged). */
  missing: boolean;
  /** Latest level frame; `undefined` until the first one arrives. */
  frame: LevelFrame | undefined;
  /** Why enumeration or metering failed (the shell's message), if it did. */
  error: string | undefined;
}

/** The enumeration failure in the default locale; the hook itself uses the mounted translator. */
export const NO_INPUT_DEVICE = zhT.t("audio.noInput");

/** Streams the native input level for `deviceId` (default device when `undefined`) while `active`.
 *  Frames arrive from Rust through a Tauri Channel (`Backend.meter`); the last one is kept in
 *  React state so the LED meter re-renders at the meter's own cadence. */
export function useAudioMeter(active: boolean, deviceId?: string): AudioMeterState {
  const { backend } = useBackend();
  const t = useT();
  const [devices, setDevices] = useState<AudioDevice[] | undefined>(undefined);
  const [frame, setFrame] = useState<LevelFrame | undefined>(undefined);
  const [error, setError] = useState<string | undefined>(undefined);

  useEffect(() => {
    let alive = true;
    backend
      .audioDevices()
      .then((list) => {
        if (!alive) return;
        setDevices(list);
        if (list.length === 0) setError(t("audio.noInput"));
      })
      .catch((e: unknown) => {
        if (alive) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      alive = false;
    };
  }, [backend, t]);

  useEffect(() => {
    if (!active) return;
    let alive = true;
    let stop: (() => void) | undefined;
    backend
      .meter(deviceId, (f) => {
        if (alive) setFrame(f);
      })
      .then((unsubscribe) => {
        if (alive) stop = unsubscribe;
        else unsubscribe();
      })
      .catch((e: unknown) => {
        if (alive) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      alive = false;
      stop?.();
      setFrame(undefined);
    };
  }, [backend, active, deviceId]);

  const fallback = devices?.find((d) => d.is_default) ?? devices?.[0];
  const chosen = deviceId === undefined ? undefined : devices?.find((d) => d.id === deviceId);
  const missing = deviceId !== undefined && devices !== undefined && chosen === undefined;
  const device = chosen ?? fallback;
  // Share the device with the title bar's microphone readout (an external store, so no prop drilling).
  useEffect(() => {
    publishMicrophone({ device, error });
  }, [device, error]);
  return { devices, device, missing, frame, error };
}

/** dBFS → 0…1 for a bar meter (−60 dBFS is silence, 0 dBFS full scale). */
export function levelFraction(dbfs: number): number {
  return Math.min(1, Math.max(0, (dbfs + 60) / 60));
}
