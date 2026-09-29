import {
  ACTIVATIONS,
  type Activation,
  DEFAULT_EDIT_HOTKEY,
  DEFAULT_HOTKEY,
  type HotkeyCapabilities,
  type HotkeyStatus,
  MAX_ACTIVATION_MS,
  type Platform,
  type Settings,
  type SoloKey,
  type TFunction,
  activationDescription,
  activationHint,
  activationLabel,
  hotkeyMethodText,
  platformLabel,
} from "@voltip/shared";
import {
  Banner,
  Button,
  CardGrid,
  IconButton,
  type IconName,
  Input,
  Keycaps,
  LampText,
  OptionCard,
  Select,
  SettingsPane,
  SettingsRows,
  SettingsSection,
  StatusRow,
} from "@voltip/ui";
import { useBackend, useI18n, useUiState } from "@voltip/ui";
import { type KeyboardEvent, useCallback, useState } from "react";
import { copyWithToast, useShell } from "../../app/shell-context";
import { useChordRecorder } from "../../features/hotkey/useChordRecorder";

/** The two activation timings (docs/dictation.md §13) as the settings pane edits them. */
export const ACTIVATION_STEP_MS = 50;
export const HOLD_THRESHOLD_RANGE = { min: 50, max: MAX_ACTIVATION_MS } as const;
export const EXTRA_RECORDING_RANGE = { min: 0, max: MAX_ACTIVATION_MS } as const;

/** Snap a typed value to the 50 ms grid inside `[min, max]`; a non-number keeps `fallback`. */
export function clampActivationMs(
  raw: string,
  range: { min: number; max: number },
  fallback: number,
): number {
  const n = Number.parseFloat(raw);
  if (!Number.isFinite(n)) return fallback;
  const snapped = Math.round(n / ACTIVATION_STEP_MS) * ACTIVATION_STEP_MS;
  return Math.min(range.max, Math.max(range.min, snapped));
}

const ACTIVATION_ICONS: Readonly<Record<Activation, IconName>> = {
  hold: "mic",
  toggle: "play",
  hold_or_toggle: "lock",
};

type ActivationSettings = Pick<Settings, "activation" | "hold_threshold_ms" | "extra_recording_ms">;

/** What the hotkey can do in this session (docs/dictation.md §13, §14), as the shell measured it,
 *  and the command a desktop or compositor shortcut runs instead of a registered chord. */
function SessionCapabilities({ capabilities }: { capabilities: HotkeyCapabilities }) {
  const shell = useShell();
  const { t } = useI18n();
  const copy = (text: string) => {
    void copyWithToast(shell, text, t("settings.hotkey.capabilities.copied"));
  };
  const everywhere = !capabilities.global
    ? { tone: "idle" as const, text: t("settings.hotkey.capabilities.everywhereNone") }
    : capabilities.everywhere
      ? { tone: "ok" as const, text: t("settings.hotkey.capabilities.everywhereYes") }
      : { tone: "warn" as const, text: t("settings.hotkey.capabilities.everywhereX11Only") };
  return (
    <SettingsSection
      title={t("settings.hotkey.capabilities.title")}
      description={t("settings.hotkey.capabilities.description")}
      data-testid="hotkey-capabilities">
      <SettingsRows>
        <StatusRow label={t("settings.hotkey.capabilities.global")} data-testid="capability-global">
          <LampText tone={capabilities.global ? "ok" : "danger"} size="sm">
            {capabilities.global
              ? t("settings.hotkey.capabilities.globalYes")
              : t("settings.hotkey.capabilities.globalNo")}
          </LampText>
        </StatusRow>
        <StatusRow
          label={t("settings.hotkey.capabilities.everywhere")}
          data-testid="capability-everywhere">
          <LampText tone={everywhere.tone} size="sm">
            {everywhere.text}
          </LampText>
        </StatusRow>
        <StatusRow label={t("settings.hotkey.capabilities.hold")} data-testid="capability-hold">
          <LampText tone={capabilities.hold ? "ok" : "idle"} size="sm">
            {capabilities.hold
              ? t("settings.hotkey.capabilities.holdYes")
              : t("settings.hotkey.capabilities.holdNo")}
          </LampText>
        </StatusRow>
        <StatusRow
          label={t("settings.hotkey.capabilities.command")}
          help={t("settings.hotkey.capabilities.commandHelp")}
          data-testid="capability-command">
          <div className="flex max-w-[360px] items-center gap-2">
            <code className="mono truncate text-[12px] text-fg" title={capabilities.toggle_command}>
              {capabilities.toggle_command}
            </code>
            <IconButton
              icon="copy"
              label={t("settings.hotkey.capabilities.copyDictation")}
              onClick={() => {
                copy(capabilities.toggle_command);
              }}
            />
          </div>
          <div className="flex max-w-[360px] items-center gap-2">
            <span className="text-[11px] text-fg-subtle">
              {t("settings.hotkey.capabilities.editCommand")}
            </span>
            <code
              className="mono truncate text-[11px] text-fg-muted"
              title={capabilities.edit_toggle_command}>
              {capabilities.edit_toggle_command}
            </code>
            <IconButton
              icon="copy"
              label={t("settings.hotkey.capabilities.copyEdit")}
              onClick={() => {
                copy(capabilities.edit_toggle_command);
              }}
            />
          </div>
        </StatusRow>
      </SettingsRows>
    </SettingsSection>
  );
}

/** How the settings page names a lone key on `platform` (the modifiers carry their platform's
 *  name: Option and Command on macOS, Win on Windows, Super elsewhere). */
export function soloKeyLabel(key: SoloKey, platform: Platform | undefined, t: TFunction): string {
  switch (key) {
    case "right_alt":
      return t(
        platform === "macos"
          ? "settings.hotkey.solo.key.right_alt_mac"
          : "settings.hotkey.solo.key.right_alt",
      );
    case "right_meta":
      return t(
        platform === "macos"
          ? "settings.hotkey.solo.key.right_meta_mac"
          : platform === "windows"
            ? "settings.hotkey.solo.key.right_meta_windows"
            : "settings.hotkey.solo.key.right_meta_linux",
      );
    default:
      return t(`settings.hotkey.solo.key.${key}`);
  }
}

/** What to know about `key` before relying on it (docs/dictation.md §13.1). */
export function soloKeyNotes(
  key: SoloKey | null,
  platform: Platform | undefined,
  everywhere: boolean,
  t: TFunction,
): string[] {
  if (key === null) return [];
  const notes: string[] = [];
  if (key === "right_alt" && platform !== "macos") notes.push(t("settings.hotkey.solo.note.alt"));
  if (key === "right_shift") notes.push(t("settings.hotkey.solo.note.shift"));
  if (key === "fn") notes.push(t("settings.hotkey.solo.note.fn"));
  if (key.startsWith("mouse_")) notes.push(t("settings.hotkey.solo.note.mouse"));
  if (platform === "macos") notes.push(t("settings.hotkey.solo.note.macPermission"));
  if (!everywhere && platform === "linux") notes.push(t("settings.hotkey.solo.note.x11Only"));
  return notes;
}

/** The lone-key trigger (docs/dictation.md §13.1): `settings.solo_key`, saved through
 *  `settings_set_solo_key`; the keys come from the shell (`capabilities.solo_keys`), and so does
 *  what its input hook watches (`solo_registered`, `solo_error`, `solo_pressed`). */
function SoloKeyRow({
  status,
  soloKey,
  platform,
}: {
  status: HotkeyStatus;
  soloKey: SoloKey | null;
  platform: Platform | undefined;
}) {
  const { backend } = useBackend();
  const { t } = useI18n();
  const offered = status.capabilities?.solo_keys ?? [];
  // A key the session cannot watch (Fn on Windows) stays visible while it is the setting.
  const keys = soloKey !== null && !offered.includes(soloKey) ? [...offered, soloKey] : offered;
  const options = [
    { value: "off" as const, label: t("settings.hotkey.solo.off") },
    ...keys.map((key) => ({ value: key, label: soloKeyLabel(key, platform, t) })),
  ];
  const watching = soloKey !== null && status.solo_registered === soloKey;
  const lamp =
    soloKey === null
      ? undefined
      : status.solo_error !== undefined
        ? { tone: "danger" as const, text: t("settings.hotkey.solo.failed") }
        : watching
          ? status.solo_pressed === true
            ? { tone: "ok" as const, text: t("settings.hotkey.solo.pressed") }
            : { tone: "ok" as const, text: t("settings.hotkey.solo.watching") }
          : { tone: "idle" as const, text: t("settings.hotkey.solo.waiting") };
  const notes = soloKeyNotes(soloKey, platform, status.capabilities?.everywhere ?? true, t);
  return (
    <StatusRow
      label={t("settings.hotkey.solo.label")}
      help={t("settings.hotkey.solo.help")}
      data-testid="solo-key"
      note={
        offered.length === 0 ? (
          t("settings.hotkey.solo.unavailable")
        ) : status.solo_error !== undefined || notes.length > 0 ? (
          <span className="flex max-w-[360px] flex-col gap-1" data-testid="solo-key-notes">
            {status.solo_error !== undefined && (
              <span role="alert" className="text-danger">
                {status.solo_error}
              </span>
            )}
            {notes.map((note) => (
              <span key={note}>{note}</span>
            ))}
          </span>
        ) : undefined
      }>
      <div className="flex items-center gap-3">
        {lamp !== undefined && (
          <LampText tone={lamp.tone} pulse={status.solo_pressed === true} size="sm">
            <span data-testid="solo-key-status">{lamp.text}</span>
          </LampText>
        )}
        <Select
          size="sm"
          className="w-48"
          aria-label={t("settings.hotkey.solo.select")}
          options={options}
          value={soloKey ?? "off"}
          disabled={offered.length === 0 && soloKey === null}
          onChange={(value) => {
            void backend.invoke("settings_set_solo_key", { key: value === "off" ? null : value });
          }}
        />
      </div>
    </StatusRow>
  );
}

/** Settings · Hotkey: backend readout, recorder, activation mode with its two timings, conflict
 *  banner. The chord is the core's `settings.hotkey` (saved through
 *  `settings_set_hotkey`); the backend, registration result and pressed state come from the shell's
 *  `hotkey` status, never from a fixture. Activation (docs/dictation.md §13) is the core's
 *  `settings.activation` / `hold_threshold_ms` / `extra_recording_ms`, written as one
 *  `settings_set_activation` whenever any of the three changes. The voice-edit chord (§19) is
 *  `settings.edit_hotkey` with its own recorder (`settings_set_edit_hotkey`, `null` = off) and the
 *  shell's `edit_registered` / `edit_error`; it needs the AI refine service, which the row says. */
export function Hotkey() {
  const shell = useShell();
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const state = useUiState();
  const hotkey = state.settings.hotkey;
  const status = state.hotkey;
  const platform = state.identity?.platform;
  const registered = status.registered === hotkey;
  const activation: ActivationSettings = {
    activation: state.settings.activation,
    hold_threshold_ms: state.settings.hold_threshold_ms,
    extra_recording_ms: state.settings.extra_recording_ms,
  };
  const setActivation = (patch: Partial<ActivationSettings>) => {
    const next = { ...activation, ...patch };
    void backend.invoke("settings_set_activation", {
      activation: next.activation,
      holdThresholdMs: next.hold_threshold_ms,
      extraRecordingMs: next.extra_recording_ms,
    });
  };
  const save = useCallback(
    (combo: string) => {
      void backend.invoke("settings_set_hotkey", { hotkey: combo });
    },
    [backend],
  );
  const reject = useCallback(
    (reason: string) => {
      shell.toast({
        message: t("settings.hotkey.rejected", { reason }),
        duration: 5000,
        tone: "danger",
      });
    },
    [shell, t],
  );
  // Physical keys, peak set, commit on the first key-up; the shell suspends the OS registration
  // (`hotkey_capture`) for as long as the recorder is open so the bound chord reaches this window.
  const recorder = useChordRecorder({ onCommit: save, onReject: reject });
  const { recording } = recorder;
  // docs/dictation.md §19: the voice-edit chord, recorded the same way; one recorder at a time.
  const editHotkey = state.settings.edit_hotkey;
  const editRegistered = editHotkey !== null && status.edit_registered === editHotkey;
  const refineMissing = !state.engines.refine_ready;
  const saveEdit = useCallback(
    (combo: string | null) => {
      void backend.invoke("settings_set_edit_hotkey", { hotkey: combo });
    },
    [backend],
  );
  const editRecorder = useChordRecorder({ onCommit: saveEdit, onReject: reject });
  const editRecording = editRecorder.recording;
  const startRecording = () => {
    editRecorder.cancel();
    recorder.start();
  };
  const startEditRecording = () => {
    recorder.cancel();
    editRecorder.start();
  };

  const platformText = platform ? platformLabel(platform, locale) : "—";
  const method = status.backend.length > 0 ? hotkeyMethodText(status.backend) : undefined;
  return (
    <SettingsPane title={t("settings.hotkey.title")} lede={t("settings.hotkey.lede")}>
      <SettingsSection title={t("settings.hotkey.backendTitle")}>
        <div
          className="flex flex-wrap items-center gap-x-8 gap-y-2 rounded-10 bg-surface p-3 text-[12px] hairline"
          data-testid="hotkey-backend">
          <span>
            <span className="text-fg-subtle">{t("settings.hotkey.platform")} </span>
            <span className="mono text-fg">{platformText}</span>
          </span>
          {/* The method row only when it says more than the platform: the Linux session. */}
          {method !== platformText && (
            <span>
              <span className="text-fg-subtle">{t("settings.hotkey.backend")} </span>
              <span className="mono text-fg">{method ?? t("settings.hotkey.notReported")}</span>
            </span>
          )}
          <span className="flex items-center gap-2">
            <span className="text-fg-subtle">{t("settings.hotkey.registration")}</span>
            <LampText
              tone={status.error ? "danger" : registered ? "ok" : "idle"}
              pulse={status.pressed}
              size="sm">
              {status.error
                ? t("settings.hotkey.regFailed")
                : registered
                  ? status.pressed
                    ? t("settings.hotkey.regPressed")
                    : t("settings.hotkey.regOk")
                  : t("settings.hotkey.regWaiting")}
            </LampText>
          </span>
        </div>
      </SettingsSection>
      {status.capabilities !== undefined && status.capabilities.toggle_command.length > 0 && (
        <SessionCapabilities capabilities={status.capabilities} />
      )}

      <SettingsRows>
        <StatusRow label={t("settings.hotkey.shortcut")} help={t("settings.hotkey.shortcutHelp")}>
          <div
            className={`flex h-11 items-center gap-3 rounded-10 bg-surface px-3 ${recording ? "border-2 border-primary" : "hairline"}`}
            data-testid="hotkey-recorder"
            data-recording={recording ? "true" : "false"}>
            {recording ? (
              recorder.preview ? (
                <Keycaps keys={recorder.preview} />
              ) : (
                <LampText tone="ok" pulse>
                  {t("settings.hotkey.recording")}
                </LampText>
              )
            ) : (
              <>
                <Keycaps keys={hotkey} />
                <span className="text-[12px] text-fg-subtle" data-testid="hotkey-status">
                  {status.error
                    ? t("settings.hotkey.savedFailed")
                    : registered
                      ? t("settings.hotkey.savedOk")
                      : t("settings.hotkey.savedWaiting")}
                </span>
              </>
            )}
            <Button
              size="sm"
              variant={recording ? "ghost" : "primary"}
              onClick={recording ? recorder.cancel : startRecording}>
              {recording ? t("settings.hotkey.cancelRecording") : t("settings.hotkey.record")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                recorder.cancel();
                save(DEFAULT_HOTKEY);
              }}>
              {t("settings.hotkey.restore")}
            </Button>
          </div>
        </StatusRow>
        <StatusRow
          label={t("settings.hotkey.editShortcut")}
          help={t("settings.hotkey.editShortcutHelp")}
          note={refineMissing ? t("settings.hotkey.editNeedsRefine") : undefined}>
          <div
            className={`flex h-11 items-center gap-3 rounded-10 bg-surface px-3 ${editRecording ? "border-2 border-primary" : "hairline"}`}
            data-testid="edit-hotkey-recorder"
            data-recording={editRecording ? "true" : "false"}>
            {editRecording ? (
              editRecorder.preview ? (
                <Keycaps keys={editRecorder.preview} />
              ) : (
                <LampText tone="ok" pulse>
                  {t("settings.hotkey.recording")}
                </LampText>
              )
            ) : editHotkey === null ? (
              <span className="text-[12px] text-fg-subtle" data-testid="edit-hotkey-status">
                {t("settings.hotkey.editOff")}
              </span>
            ) : (
              <>
                <Keycaps keys={editHotkey} />
                <span className="text-[12px] text-fg-subtle" data-testid="edit-hotkey-status">
                  {status.edit_error
                    ? t("settings.hotkey.savedFailed")
                    : editRegistered
                      ? t("settings.hotkey.savedOk")
                      : t("settings.hotkey.savedWaiting")}
                </span>
              </>
            )}
            <Button
              size="sm"
              variant={editRecording ? "ghost" : "primary"}
              // The visible words are the dictation row's; the name says which chord it records.
              aria-label={
                editRecording
                  ? t("settings.hotkey.editCancelLabel")
                  : t("settings.hotkey.editRecordLabel")
              }
              onClick={editRecording ? editRecorder.cancel : startEditRecording}>
              {editRecording ? t("settings.hotkey.cancelRecording") : t("settings.hotkey.record")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              aria-label={
                editHotkey === null
                  ? t("settings.hotkey.editTurnOnLabel")
                  : t("settings.hotkey.editTurnOffLabel")
              }
              onClick={() => {
                editRecorder.cancel();
                saveEdit(editHotkey === null ? DEFAULT_EDIT_HOTKEY : null);
              }}>
              {editHotkey === null
                ? t("settings.hotkey.editTurnOn")
                : t("settings.hotkey.editTurnOff")}
            </Button>
          </div>
        </StatusRow>
        <SoloKeyRow status={status} soloKey={state.settings.solo_key} platform={platform} />
      </SettingsRows>

      <SettingsSection
        title={t("settings.hotkey.activation")}
        description={t("settings.hotkey.activationHelp")}
        data-testid="activation">
        <CardGrid min={200} role="listbox" aria-label={t("settings.hotkey.activation")}>
          {ACTIVATIONS.map((mode) => (
            <OptionCard
              key={mode}
              icon={ACTIVATION_ICONS[mode]}
              title={activationLabel(mode, locale)}
              aria-label={activationLabel(mode, locale)}
              selected={activation.activation === mode}
              onSelect={() => {
                if (activation.activation !== mode) setActivation({ activation: mode });
              }}>
              <p className="text-[12px] leading-4 text-fg-muted">
                {activationDescription(mode, locale)}
              </p>
            </OptionCard>
          ))}
        </CardGrid>
        <div className="mono text-[11px] text-fg-subtle" data-testid="activation-hint">
          {activationHint(activation.activation, hotkey, locale)}
        </div>
        <SettingsRows>
          {activation.activation === "hold_or_toggle" && (
            <StatusRow
              label={t("settings.hotkey.holdThreshold")}
              help={t("settings.hotkey.holdThresholdHelp")}>
              <MsField
                label={t("settings.hotkey.holdThreshold")}
                unit={t("settings.hotkey.ms")}
                value={activation.hold_threshold_ms}
                range={HOLD_THRESHOLD_RANGE}
                testId="hold-threshold"
                onCommit={(hold_threshold_ms) => {
                  setActivation({ hold_threshold_ms });
                }}
              />
            </StatusRow>
          )}
          <StatusRow
            label={t("settings.hotkey.extraRecording")}
            help={t("settings.hotkey.extraRecordingHelp")}
            note={
              activation.extra_recording_ms === 0 ? t("settings.hotkey.immediateStop") : undefined
            }>
            <MsField
              label={t("settings.hotkey.extraRecording")}
              unit={t("settings.hotkey.ms")}
              value={activation.extra_recording_ms}
              range={EXTRA_RECORDING_RANGE}
              testId="extra-recording"
              onCommit={(extra_recording_ms) => {
                setActivation({ extra_recording_ms });
              }}
            />
          </StatusRow>
        </SettingsRows>
      </SettingsSection>

      {status.error && (
        <Banner
          tone="danger"
          marker="bar"
          actions={
            <Button size="sm" onClick={startRecording}>
              {t("settings.hotkey.change")}
            </Button>
          }>
          {status.error}
        </Banner>
      )}
      {status.edit_error && (
        <Banner
          tone="danger"
          marker="bar"
          actions={
            <Button size="sm" onClick={startEditRecording}>
              {t("settings.hotkey.change")}
            </Button>
          }>
          {status.edit_error}
        </Banner>
      )}

      <div className="mono text-[11px] text-fg-subtle">
        {t("settings.hotkey.storage", { hotkey })}
      </div>
    </SettingsPane>
  );
}

interface MsFieldProps {
  label: string;
  unit: string;
  value: number;
  range: { min: number; max: number };
  testId: string;
  onCommit: (value: number) => void;
}

/** A millisecond field on the 50 ms grid: the draft is local while typing and is snapped into the
 *  range and written once on blur / Enter, so a half-typed number never reaches the core. */
function MsField({ label, unit, value, range, testId, onCommit }: MsFieldProps) {
  const [draft, setDraft] = useState<string | undefined>(undefined);
  const commit = () => {
    if (draft === undefined) return;
    const next = clampActivationMs(draft, range, value);
    setDraft(undefined);
    if (next !== value) onCommit(next);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "Enter") return;
    e.preventDefault();
    commit();
  };
  return (
    <div className="flex items-center gap-2">
      <Input
        type="number"
        inputMode="numeric"
        mono
        size="sm"
        className="w-24"
        aria-label={label}
        data-testid={testId}
        min={range.min}
        max={range.max}
        step={ACTIVATION_STEP_MS}
        value={draft ?? String(value)}
        onChange={(e) => {
          setDraft(e.target.value);
        }}
        onBlur={commit}
        onKeyDown={onKeyDown}
      />
      <span className="text-[12px] text-fg-muted">{unit}</span>
    </div>
  );
}
