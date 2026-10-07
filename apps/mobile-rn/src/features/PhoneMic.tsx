// The phone's talk card (docs/dictation.md §20, §20.7), apps/mobile's PhoneMic on native views:
// hold to talk, release to finish, slide off the button before releasing to cancel. With a paired
// computer online the take streams to it, and the computer recognises and inserts the text; with
// none online the phone recognises it itself through the built-in service and copies the result.
// A take keeps its route until it ends.
import {
  type DeviceView,
  type DictationPhase,
  type LevelFrame,
  type PhoneTakeState,
  type TFunction,
  coreMessageText,
  errorText,
  formatElapsed,
  phoneTakeFinal,
  sceneLabel,
} from "@voltip/shared";
import * as Haptics from "expo-haptics";
import { useEffect, useRef, useState } from "react";
import { Animated, type GestureResponderEvent, type LayoutRectangle, View } from "react-native";
import { Icon, Text } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useShell } from "../shell";
import { Meter, Mono, Section, StateLine, type Tone, useAppTheme } from "../ui/kit";
import { SelectField } from "../ui/Select";

/** How often the listening timer redraws. */
const TICK_MS = 250;

/** The line under the button: where a take sent to a computer is. */
export function phoneTakeLine(state: PhoneTakeState, elapsedMs: number, t: TFunction): string {
  switch (state.state) {
    case "starting":
      return t("mobile.mic.starting");
    case "listening":
      return t("mobile.mic.listening", { elapsed: formatElapsed(elapsedMs) });
    case "processing":
      return t("mobile.mic.processing");
    case "done":
      return t(state.pasted ? "mobile.mic.donePasted" : "mobile.mic.doneClipboard", {
        text: state.text,
      });
    case "failed":
      return t(`mobile.mic.failed.${state.code}`, { message: state.message });
    case "cancelled":
      return t("mobile.mic.cancelled");
  }
}

/** The line under the button for a take the phone recognises itself (docs/dictation.md §20.7). */
export function localTakeLine(phase: DictationPhase, elapsedMs: number, t: TFunction): string {
  switch (phase.phase) {
    case "idle":
      return "";
    case "listening":
      return t("mobile.mic.listening", { elapsed: formatElapsed(elapsedMs) });
    case "processing":
      if (phase.stage === "transcribing") return t("mobile.mic.local.transcribing");
      if (phase.stage === "refining") return t("mobile.mic.local.refining");
      return t("mobile.mic.local.processing");
    case "done":
      return t("mobile.mic.local.copied", { text: phase.text });
    case "failed":
      return phase.code === "no_speech"
        ? t("mobile.mic.failed.no_speech")
        : t("mobile.mic.local.failed", { message: coreMessageText(phase.message) });
    case "cancelled":
      return t("mobile.mic.cancelled");
  }
}

function takeTone(state: PhoneTakeState): Tone {
  switch (state.state) {
    case "listening":
      return "ok";
    case "starting":
    case "processing":
      return "accent";
    case "failed":
      return "danger";
    default:
      return "idle";
  }
}

function localTone(phase: DictationPhase): Tone {
  switch (phase.phase) {
    case "listening":
      return "ok";
    case "processing":
      return "accent";
    case "failed":
      return "danger";
    default:
      return "idle";
  }
}

/** dBFS → 0…1 for the meter (−60 dBFS is silence, 0 dBFS full scale). */
export function levelFraction(dbfs: number): number {
  return Math.min(1, Math.max(0, (dbfs + 60) / 60));
}

/** The level of the phone's own take while it records: the take's capture feeds it, so the meter
 *  never opens the microphone by itself. */
function useTakeLevel(active: boolean): LevelFrame | undefined {
  const { backend } = useBackend();
  const [frame, setFrame] = useState<LevelFrame | undefined>(undefined);
  useEffect(() => {
    if (!active) return;
    let alive = true;
    let stop: (() => void) | undefined;
    backend
      .meter(undefined, (f) => {
        if (alive) setFrame(f);
      })
      .then(
        (unsubscribe) => {
          if (alive) stop = unsubscribe;
          else unsubscribe();
        },
        // No meter is no reason to stop the take; the line below still follows it.
        () => undefined,
      );
    return () => {
      alive = false;
      stop?.();
      setFrame(undefined);
    };
  }, [backend, active]);
  return frame;
}

function useTicking(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => {
      setNow(Date.now());
    }, TICK_MS);
    return () => {
      clearInterval(id);
    };
  }, [active]);
  return now;
}

/** A start the app refused before the core saw it (the microphone permission was denied): a
 *  toast, and the button lets go. */
function useStart(): (run: () => Promise<void>) => Promise<boolean> {
  const shell = useShell();
  const { t } = useI18n();
  return (run) =>
    run().then(
      () => true,
      (e: unknown) => {
        shell.toast(
          t("mobile.toast.error", {
            message: coreMessageText(e instanceof Error ? e.message : String(e)),
          }),
          "danger",
        );
        return false;
      },
    );
}

const BUTTON = 136;

/** The hold-to-talk button both routes share. `start` resolves whether the take started; a release
 *  waits for it, so a quick tap still stops the take it started. The button keeps the gesture
 *  while held (the page does not scroll away under the finger); a release outside it cancels. */
function HoldButton({
  busy,
  sublabel,
  releaseLabel,
  level,
  start,
  stop,
  cancel,
}: {
  busy: boolean;
  sublabel: string;
  releaseLabel: string;
  level: number;
  start: () => Promise<boolean>;
  stop: () => void;
  cancel: () => void;
}) {
  const theme = useAppTheme();
  const { t } = useI18n();
  const [held, setHeld] = useState(false);
  const [offButton, setOffButton] = useState(false);
  const started = useRef<Promise<boolean> | null>(null);
  const frame = useRef<LayoutRectangle>({ x: 0, y: 0, width: BUTTON, height: BUTTON });
  const [scale] = useState(() => new Animated.Value(1));
  const [ring] = useState(() => new Animated.Value(0));

  useEffect(() => {
    Animated.spring(scale, {
      toValue: held ? 1.08 : 1,
      useNativeDriver: true,
      speed: 30,
      bounciness: 6,
    }).start();
  }, [held, scale]);
  useEffect(() => {
    Animated.timing(ring, {
      toValue: held ? level : 0,
      duration: 90,
      useNativeDriver: true,
    }).start();
  }, [held, level, ring]);

  const outside = (e: GestureResponderEvent) => {
    const { locationX: x, locationY: y } = e.nativeEvent;
    const { width, height } = frame.current;
    // A margin around the button, as the platform's press retention gives one.
    return x < -24 || y < -24 || x > width + 24 || y > height + 24;
  };
  const begin = () => {
    if (busy || started.current !== null) return;
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    setHeld(true);
    setOffButton(false);
    started.current = start().then((ok) => {
      if (!ok) {
        setHeld(false);
        setOffButton(false);
      }
      return ok;
    });
  };
  const finish = (cancelled: boolean) => {
    const pending = started.current;
    started.current = null;
    setHeld(false);
    setOffButton(false);
    if (pending === null) return;
    void (cancelled
      ? Haptics.notificationAsync(Haptics.NotificationFeedbackType.Warning)
      : Haptics.selectionAsync());
    void pending.then((ok) => {
      if (ok) (cancelled ? cancel : stop)();
    });
  };

  const label = !held
    ? t("mobile.mic.hold")
    : offButton
      ? t("mobile.mic.releaseCancel")
      : releaseLabel;
  // The page's one solid accent; while held it turns to the brighter container, and over a release
  // that cancels to the danger shade.
  const fill = offButton
    ? theme.voltip.dangerSoft
    : held
      ? theme.colors.primaryContainer
      : theme.colors.primary;
  const ink = offButton
    ? theme.voltip.danger
    : held
      ? theme.colors.onPrimaryContainer
      : theme.colors.onPrimary;
  const disabled = busy && !held;
  return (
    <View style={{ alignItems: "center", gap: 12, paddingVertical: 8 }}>
      <View
        style={{
          width: BUTTON + 40,
          height: BUTTON + 40,
          alignItems: "center",
          justifyContent: "center",
        }}>
        <Animated.View
          pointerEvents="none"
          style={{
            position: "absolute",
            width: BUTTON + 40,
            height: BUTTON + 40,
            borderRadius: (BUTTON + 40) / 2,
            backgroundColor: theme.colors.primary,
            opacity: ring.interpolate({ inputRange: [0, 1], outputRange: [0, 0.28] }),
            transform: [{ scale: ring.interpolate({ inputRange: [0, 1], outputRange: [0.8, 1] }) }],
          }}
        />
        <Animated.View
          testID="phone-mic-hold"
          accessible
          accessibilityRole="button"
          accessibilityLabel={label}
          accessibilityHint={sublabel}
          accessibilityState={{ disabled, selected: held }}
          onLayout={(e) => {
            frame.current = e.nativeEvent.layout;
          }}
          onStartShouldSetResponder={() => !disabled}
          onResponderTerminationRequest={() => false}
          onResponderGrant={begin}
          onResponderMove={(e) => {
            if (held) setOffButton(outside(e));
          }}
          onResponderRelease={(e) => {
            finish(outside(e));
          }}
          onResponderTerminate={() => {
            finish(true);
          }}
          style={{
            width: BUTTON,
            height: BUTTON,
            borderRadius: BUTTON / 2,
            backgroundColor: fill,
            alignItems: "center",
            justifyContent: "center",
            gap: 4,
            opacity: disabled ? 0.5 : 1,
            // A shadow in the accent's own colour (Android 9+ tints elevation shadows).
            elevation: held ? 10 : 6,
            shadowColor: offButton ? theme.voltip.danger : theme.colors.primary,
            transform: [{ scale }],
          }}>
          <Icon source={offButton ? "close" : "microphone"} size={40} color={ink} />
          <Text
            variant="labelLarge"
            style={{ color: ink, textAlign: "center", paddingHorizontal: 12 }}
            numberOfLines={2}>
            {label}
          </Text>
        </Animated.View>
      </View>
      <Text
        variant="bodySmall"
        testID="phone-mic-route"
        style={{ color: theme.colors.onSurfaceVariant, textAlign: "center" }}>
        {sublabel}
      </Text>
    </View>
  );
}

/** Where a take stands, under the button: a status line that wraps, the codec while it streams,
 *  and the level while it records. */
function TakeState({
  state,
  tone,
  pulse,
  line,
  codec,
  level,
}: {
  state: string;
  tone: Tone;
  pulse: boolean;
  line: string;
  codec?: string | undefined;
  level?: { fraction: number; peak: number | undefined } | undefined;
}) {
  const { t } = useI18n();
  return (
    <View style={{ gap: 8 }} testID="phone-mic-state" accessibilityLabel={state}>
      <StateLine tone={tone} pulse={pulse} numberOfLines={3}>
        {line}
      </StateLine>
      {codec !== undefined && <Mono>{codec}</Mono>}
      {level !== undefined && (
        <Meter
          label={t("mobile.mic.level")}
          level={level.fraction}
          {...(level.peak === undefined ? {} : { peak: level.peak })}
          segments={40}
        />
      )}
    </View>
  );
}

function meterOf(frame: LevelFrame | undefined) {
  return {
    fraction: frame === undefined ? 0 : levelFraction(frame.rms_dbfs),
    peak: frame === undefined ? undefined : levelFraction(frame.peak_dbfs),
  };
}

/** The take streams to a paired computer, which recognises and inserts the text. */
function ComputerTalk({
  desktops,
  online,
}: {
  desktops: readonly DeviceView[];
  online: readonly DeviceView[];
}) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const { t } = useI18n();
  const { phone_take: take } = useUiState();
  const begin = useStart();
  const [picked, setPicked] = useState<string | undefined>(undefined);
  const running = take !== undefined && !phoneTakeFinal(take.state);
  // A running take stays with its computer even when that one went offline meanwhile.
  const target =
    (running ? desktops.find((d) => d.device.public_key === take.device) : undefined) ??
    online.find((d) => d.device.public_key === picked) ??
    online[0];
  const listening = take?.state.state === "listening";
  const now = useTicking(listening);
  const frame = useTakeLevel(listening);
  if (target === undefined) return null;
  const meter = meterOf(frame);
  return (
    <Section title={t("mobile.mic.title")} padded testID="phone-mic">
      <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
        {t("mobile.mic.body")}
      </Text>
      {online.length > 1 && (
        <SelectField
          label={t("mobile.mic.target")}
          value={target.device.public_key}
          disabled={running}
          options={online.map((d) => ({ value: d.device.public_key, label: d.device.name }))}
          onChange={setPicked}
          testID="phone-mic-target"
        />
      )}
      <HoldButton
        busy={running}
        level={listening ? meter.fraction : 0}
        sublabel={t("mobile.mic.toDesktop", { name: target.device.name })}
        releaseLabel={t("mobile.mic.release")}
        start={() =>
          begin(() => backend.invoke("phone_take_start", { publicKey: target.device.public_key }))
        }
        stop={() => void backend.invoke("phone_take_stop")}
        cancel={() => void backend.invoke("phone_take_cancel")}
      />
      {take !== undefined && take.device === target.device.public_key && (
        <TakeState
          state={take.state.state}
          tone={takeTone(take.state)}
          pulse={take.state.state === "listening"}
          line={phoneTakeLine(take.state, now - take.started_at, t)}
          codec={
            take.opus === true && !phoneTakeFinal(take.state)
              ? t("mobile.mic.codecOpus")
              : undefined
          }
          level={take.state.state === "listening" ? meter : undefined}
        />
      )}
    </Section>
  );
}

/** The scene the phone's takes run with (docs/dictation.md §18; user decision 2026-10-01): the
 *  phone cannot tell which app the text goes to, so the user picks one, or none. */
function ScenePicker({ disabled }: { disabled: boolean }) {
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const { scenes, settings } = useUiState();
  if (scenes.length === 0) return null;
  return (
    <SelectField
      label={t("mobile.mic.scene")}
      value={settings.pinned_scene ?? ""}
      disabled={disabled}
      testID="phone-scene"
      options={[
        { value: "", label: t("mobile.mic.noScene") },
        ...scenes.map((scene) => ({ value: scene.id, label: sceneLabel(scene, locale) })),
      ]}
      onChange={(id) => {
        backend
          .invoke("settings_set_pinned_scene", { id: id === "" ? null : id })
          .catch((e: unknown) => {
            shell.toast(t("mobile.toast.error", { message: errorText(e) }), "danger");
          });
      }}
    />
  );
}

/** The phone recognises the take itself (docs/dictation.md §20.7): the built-in service
 *  transcribes and polishes it, and the result lands on the phone's clipboard. */
function PhoneTalk({ paired }: { paired: boolean }) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const { t } = useI18n();
  const { dictation } = useUiState();
  const begin = useStart();
  const phase = dictation.phase;
  const running = phase.phase === "listening" || phase.phase === "processing";
  const listening = phase.phase === "listening";
  const now = useTicking(listening);
  const frame = useTakeLevel(listening);
  const meter = meterOf(frame);
  const line = localTakeLine(phase, listening && phase.ready ? now - phase.started_at : 0, t);
  return (
    <Section title={t("mobile.mic.title")} padded testID="phone-mic">
      <View style={{ gap: 4 }}>
        <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
          {t("mobile.mic.localBody")}
        </Text>
        {paired && (
          <Text
            variant="bodySmall"
            style={{ color: theme.colors.onSurfaceVariant }}
            testID="phone-mic-offline">
            {t("mobile.mic.offline")}
          </Text>
        )}
      </View>
      <ScenePicker disabled={running} />
      <HoldButton
        busy={running}
        level={listening ? meter.fraction : 0}
        sublabel={t("mobile.mic.onPhone")}
        releaseLabel={t("mobile.mic.releaseLocal")}
        start={() => begin(() => backend.invoke("dictation_start"))}
        stop={() => void backend.invoke("dictation_stop")}
        cancel={() => void backend.invoke("dictation_cancel")}
      />
      {line !== "" && (
        <TakeState
          state={phase.phase}
          tone={localTone(phase)}
          pulse={listening}
          line={line}
          level={listening ? meter : undefined}
        />
      )}
    </Section>
  );
}

/** Which route a take takes: to a computer when one is online (or a take to one still runs), the
 *  phone's own recognition otherwise. */
export function PhoneMic({ desktops }: { desktops: readonly DeviceView[] }) {
  const { phone_take: take, dictation } = useUiState();
  const online = desktops.filter((d) => d.connection.state === "online");
  const toComputer = take !== undefined && !phoneTakeFinal(take.state);
  const onPhone = dictation.phase.phase === "listening" || dictation.phase.phase === "processing";
  const route = toComputer ? "computer" : onPhone || online.length === 0 ? "phone" : "computer";
  return route === "computer" ? (
    <ComputerTalk desktops={desktops} online={online} />
  ) : (
    <PhoneTalk paired={desktops.length > 0} />
  );
}
