import { type AudioDevice, type TFunction, zhT } from "@voltip/shared";
import { useSyncExternalStore } from "react";

export interface MicrophoneReadout {
  /** Device the meter is on (or the default input), once enumerated. */
  device: AudioDevice | undefined;
  /** Enumeration / open failure reported by the native backend. */
  error: string | undefined;
}

// One in-memory copy shared by the home page's microphone card (which drives the native meter)
// and the title bar's microphone readout, so both name the same real device.
let current: MicrophoneReadout = { device: undefined, error: undefined };
const listeners = new Set<() => void>();

export function publishMicrophone(next: MicrophoneReadout): void {
  if (next.device === current.device && next.error === current.error) return;
  current = next;
  for (const l of listeners) l();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function snapshot(): MicrophoneReadout {
  return current;
}

/** The microphone the app is metering, for readouts outside the home page. */
export function useMicrophoneReadout(): MicrophoneReadout {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

/** Title-bar value: the device's short name, the failure, or a pending marker. */
export function microphoneReadoutValue(readout: MicrophoneReadout, t: TFunction = zhT.t): string {
  if (readout.device) return shortMicrophoneName(readout.device.name);
  if (readout.error) return t("audio.unavailable");
  return t("audio.enumerating");
}

/** The title bar has room for a short device name: drop parentheticals and generic suffixes
 *  ("USB Microphone", "Microphone", "sound server"), collapse whitespace, and cap at 28 characters
 *  ("Playback/recording through the PulseAudio sound server" used to fill the whole title bar). */
export const MICROPHONE_READOUT_MAX = 28;

/** Removes every `( … )` group, nested ones included ("Array (Realtek(R) Audio)"). */
function stripParentheticals(name: string): string {
  let out = "";
  let depth = 0;
  for (const ch of name) {
    if (ch === "(") depth += 1;
    else if (ch === ")") depth = Math.max(0, depth - 1);
    else if (depth === 0) out += ch;
  }
  return out;
}

export function shortMicrophoneName(name: string): string {
  const cleaned = stripParentheticals(name)
    .replace(/\s+(USB\s+)?Microphone$/iu, "")
    .replace(/\s+sound server$/iu, "")
    .replace(/^Playback\/recording through the\s+/iu, "")
    .replace(/\s+/gu, " ")
    .trim();
  const base = cleaned === "" ? name.trim() : cleaned;
  return base.length > MICROPHONE_READOUT_MAX
    ? `${base.slice(0, MICROPHONE_READOUT_MAX - 1).trimEnd()}…`
    : base;
}

/** Tests: forget the published device. */
export function resetMicrophoneReadout(): void {
  publishMicrophone({ device: undefined, error: undefined });
}
