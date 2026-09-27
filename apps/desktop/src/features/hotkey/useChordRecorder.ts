import { type TFunction, zhT } from "@voltip/shared";
import { useBackend, useT } from "@voltip/ui";
import { useCallback, useEffect, useRef, useState } from "react";

/** `KeyboardEvent.code` → chord part. Modifiers collapse left/right; letters and digits lose their
 *  `Key` / `Digit` prefix so the core's canonical form (`Ctrl+Shift+D`) comes out directly; every
 *  other physical key keeps its code name (`Space`, `F5`, `Comma`), which the core passes through. */
export function chordPartFromCode(code: string): string | undefined {
  if (code === "") return undefined;
  if (code.startsWith("Control")) return "Ctrl";
  if (code.startsWith("Alt")) return "Alt";
  if (code.startsWith("Shift")) return "Shift";
  if (code.startsWith("Meta") || code === "OSLeft" || code === "OSRight") return "Meta";
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);
  return code;
}

const MODIFIER_ORDER = ["Ctrl", "Alt", "Shift", "Meta"] as const;

export function isModifierPart(part: string): boolean {
  return (MODIFIER_ORDER as readonly string[]).includes(part);
}

/** Canonical chord text for a set of physical codes: modifiers in the core's order, then the key. */
export function chordFromCodes(codes: Iterable<string>): string {
  const parts = new Set<string>();
  for (const code of codes) {
    const part = chordPartFromCode(code);
    if (part !== undefined) parts.add(part);
  }
  const modifiers = MODIFIER_ORDER.filter((m) => parts.has(m));
  const keys = [...parts].filter((p) => !isModifierPart(p));
  return [...modifiers, ...keys].join("+");
}

/** Why a chord is refused before it is sent to the core (mirrors `voltip_core::HotkeyError`). */
export function chordProblem(chord: string, t: TFunction = zhT.t): string | undefined {
  const parts = chord.split("+").filter((p) => p.length > 0);
  const keys = parts.filter((p) => !isModifierPart(p));
  if (keys.length === 0) return t("chord.modifiersOnly");
  if (keys.length > 1) return t("chord.oneKey");
  if (parts.length === keys.length) return t("chord.needModifier");
  return undefined;
}

export interface ChordRecorder {
  recording: boolean;
  /** Chord held right now (the peak set), for the live preview. */
  preview: string | undefined;
  start: () => void;
  cancel: () => void;
}

export interface ChordRecorderOptions {
  /** A complete, valid chord was released. */
  onCommit: (chord: string) => void;
  /** The released chord was refused (reason in Chinese). */
  onReject?: (reason: string) => void;
}

/** Records a keyboard chord the way the shell will register it: physical keys (`event.code`), the
 *  peak set of keys held together, committed on the first `keyup`; Esc or losing the window cancels.
 *  While recording, the shell suspends the OS registration (`hotkey_capture`) so the currently bound
 *  chord reaches this window instead of being swallowed by `RegisterHotKey`. */
export function useChordRecorder({ onCommit, onReject }: ChordRecorderOptions): ChordRecorder {
  const { backend } = useBackend();
  const t = useT();
  const [recording, setRecording] = useState(false);
  const [preview, setPreview] = useState<string | undefined>(undefined);
  const pressed = useRef(new Set<string>());
  const peak = useRef(new Set<string>());
  const committed = useRef(false);

  const reset = useCallback(() => {
    pressed.current.clear();
    peak.current.clear();
    committed.current = false;
    setPreview(undefined);
  }, []);

  const start = useCallback(() => {
    reset();
    setRecording(true);
  }, [reset]);

  const cancel = useCallback(() => {
    reset();
    setRecording(false);
  }, [reset]);

  // Suspend / restore the OS registration for exactly as long as the recorder is open.
  useEffect(() => {
    if (!recording) return;
    void backend.invoke("hotkey_capture", { active: true });
    return () => {
      void backend.invoke("hotkey_capture", { active: false });
    };
  }, [recording, backend]);

  useEffect(() => {
    if (!recording) return;
    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        cancel();
        return;
      }
      if (e.repeat || committed.current || pressed.current.has(e.code)) return;
      pressed.current.add(e.code);
      peak.current.add(e.code);
      setPreview(chordFromCodes(peak.current));
    };
    const onKeyUp = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (!pressed.current.delete(e.code) || committed.current) return;
      committed.current = true;
      const chord = chordFromCodes(peak.current);
      const problem = chordProblem(chord, t);
      if (problem === undefined) onCommit(chord);
      else onReject?.(problem);
      cancel();
    };
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("blur", cancel);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("blur", cancel);
    };
  }, [recording, cancel, onCommit, onReject, t]);

  return { recording, preview, start, cancel };
}
