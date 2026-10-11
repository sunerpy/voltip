// The phone's talk card (docs/dictation.md §20, §20.7; Penpot 「澄空 Glass · Voltip」 boards 03/04):
// hold to talk, release to finish, slide off the button before releasing to cancel. With a paired
// computer online the take streams to it, and the computer recognises and inserts the text; the
// phone can recognise it itself instead (the 识别方式 segment), and does when no computer is
// online: the built-in service transcribes it and the result is copied. A take keeps its route
// until it ends.
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
import { type ReactNode, useEffect, useRef, useState } from "react";
import {
  Animated,
  type GestureResponderEvent,
  type LayoutRectangle,
  StyleSheet,
  View,
} from "react-native";
import { Icon, Text } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useShell } from "../shell";
import {
  Hint,
  Lamp,
  Mono,
  Segmented,
  StateLine,
  type Tone,
  cardShadow,
  rgba,
  useAppTheme,
} from "../ui/kit";
import { SelectChip } from "../ui/Select";

/** How often the listening timer redraws. */
const TICK_MS = 250;

/** The line under the card's hint once a take sent to a computer is past listening. */
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

/** The line for a take the phone recognises itself (docs/dictation.md §20.7). */
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

/** dBFS → 0…1 for the wave (−60 dBFS is silence, 0 dBFS full scale). */
export function levelFraction(dbfs: number): number {
  return Math.min(1, Math.max(0, (dbfs + 60) / 60));
}

/** Bars of board 04's wave: 2 dp wide, 4 dp apart, across 198 dp. */
export const WAVE_BARS = 50;

/** The level of the phone's own take while it records, and the last [WAVE_BARS] of it for the
 *  wave: the take's capture feeds it, so the meter never opens the microphone by itself. */
function useTakeLevel(active: boolean): { frame: LevelFrame | undefined; levels: number[] } {
  const { backend } = useBackend();
  const [frame, setFrame] = useState<LevelFrame | undefined>(undefined);
  const [levels, setLevels] = useState<number[]>([]);
  useEffect(() => {
    if (!active) return;
    let alive = true;
    let stop: (() => void) | undefined;
    backend
      .meter(undefined, (f) => {
        if (!alive) return;
        setFrame(f);
        setLevels((l) => [...l.slice(1 - WAVE_BARS), levelFraction(f.rms_dbfs)]);
      })
      .then(
        (unsubscribe) => {
          if (alive) stop = unsubscribe;
          else unsubscribe();
        },
        // No meter is no reason to stop the take; the card still follows it.
        () => undefined,
      );
    return () => {
      alive = false;
      stop?.();
      setFrame(undefined);
      setLevels([]);
    };
  }, [backend, active]);
  return { frame, levels };
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

/** The talk button (board 03): 120 dp. */
const BUTTON = 120;
/** The rings around it: two at rest, three while it records, swelling with the level (board 04). */
const HALOS_IDLE = [
  { size: 180, opacity: 0.07 },
  { size: 156, opacity: 0.11 },
] as const;
const HALOS_HELD = [
  { size: 216, opacity: 0.06 },
  { size: 176, opacity: 0.1 },
  { size: 142, opacity: 0.16 },
] as const;
/** The stage the button sits on (board 03: its centre 150 dp into the card, the label at 246); the
 *  outer ring may reach past it. */
const STAGE = 192;

/** The take's level over its last seconds (board 04): the newer half in the accent, the older
 *  half faint. */
function Wave({ levels }: { levels: readonly number[] }) {
  const theme = useAppTheme();
  const { t } = useI18n();
  const last = levels.at(-1) ?? 0;
  return (
    <View
      testID="phone-mic-wave"
      accessibilityRole="progressbar"
      accessibilityLabel={t("mobile.mic.level")}
      accessibilityValue={{ min: 0, max: 100, now: Math.round(last * 100) }}
      style={styles.wave}>
      {Array.from({ length: WAVE_BARS }, (_, i) => {
        const level = levels[levels.length - WAVE_BARS + i] ?? 0;
        const hot = i >= WAVE_BARS / 2;
        return (
          <View
            key={i}
            style={{
              width: 2,
              height: Math.max(2, Math.round(level * 14) * 2),
              borderRadius: 1,
              backgroundColor: hot ? theme.colors.primary : theme.voltip.subtle,
              opacity: hot ? 1 : 0.45,
            }}
          />
        );
      })}
    </View>
  );
}

/** Where a take stands once it is past listening: a status line that wraps. */
function TakeState({ state, tone, line }: { state: string; tone: Tone; line: string }) {
  return (
    <View testID="phone-mic-state" accessibilityLabel={state} style={{ alignSelf: "center" }}>
      <StateLine tone={tone} numberOfLines={3}>
        {line}
      </StateLine>
    </View>
  );
}

/** The talk card both routes share (boards 03/04): its eyebrow (用手机说话, or the recording's
 *  time), the hold-to-talk button in its rings, the wave while it records, what releasing does and
 *  how to cancel. `start` resolves whether the take started; a release waits for it, so a quick tap
 *  still stops the take it started. The button keeps the gesture while held (the page does not
 *  scroll away under the finger); a release outside it cancels. */
function TalkCard({
  busy,
  recording,
  starting,
  level,
  levels,
  after,
  releaseLabel,
  codec,
  start,
  stop,
  cancel,
  children,
}: {
  busy: boolean;
  /** The recording's time while it records. */
  recording: string | undefined;
  starting: boolean;
  level: number;
  levels: readonly number[];
  /** What releasing does (board 03's first line of the hint). */
  after: string;
  releaseLabel: string;
  codec?: string | undefined;
  start: () => Promise<boolean>;
  stop: () => void;
  cancel: () => void;
  /** The take's state line once it is past listening. */
  children?: ReactNode;
}) {
  const theme = useAppTheme();
  const { t } = useI18n();
  const [held, setHeld] = useState(false);
  const [offButton, setOffButton] = useState(false);
  const started = useRef<Promise<boolean> | null>(null);
  const frame = useRef<LayoutRectangle>({ x: 0, y: 0, width: BUTTON, height: BUTTON });
  const [scale] = useState(() => new Animated.Value(1));
  const [swell] = useState(() => new Animated.Value(0));

  useEffect(() => {
    Animated.spring(scale, {
      toValue: held ? 1.06 : 1,
      useNativeDriver: true,
      speed: 30,
      bounciness: 6,
    }).start();
  }, [held, scale]);
  useEffect(() => {
    Animated.timing(swell, {
      toValue: held ? level : 0,
      duration: 90,
      useNativeDriver: true,
    }).start();
  }, [held, level, swell]);

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
  // The page's one solid accent, darker while held; over a release that cancels, the danger shade.
  const fill = offButton
    ? theme.voltip.dangerSoft
    : held
      ? theme.voltip.heroPressed
      : theme.voltip.hero;
  const ink = offButton ? theme.voltip.danger : theme.voltip.onHero;
  const glow = offButton ? theme.voltip.danger : theme.colors.primary;
  const disabled = busy && !held;
  const halos = held ? HALOS_HELD : HALOS_IDLE;
  return (
    <View
      testID="phone-mic"
      style={[
        styles.card,
        {
          backgroundColor: theme.voltip.card,
          borderColor: theme.voltip.cardEdge,
          borderRadius: theme.voltip.radius.hero,
        },
        cardShadow(theme, 16, 40, 0.1),
      ]}>
      <View style={styles.eyebrow}>
        {recording !== undefined || starting ? (
          <>
            <Lamp tone="accent" pulse />
            <Text
              testID="phone-mic-eyebrow"
              style={[styles.eyebrowText, { color: theme.colors.primary }]}>
              {recording !== undefined
                ? t("mobile.mic.listening", { elapsed: recording })
                : t("mobile.mic.starting")}
            </Text>
          </>
        ) : (
          <Text
            testID="phone-mic-eyebrow"
            style={[styles.eyebrowText, { color: theme.voltip.subtle }]}>
            {t("mobile.mic.title")}
          </Text>
        )}
      </View>
      <View style={styles.stage}>
        {halos.map((halo) => (
          <Animated.View
            key={halo.size}
            pointerEvents="none"
            testID="phone-mic-halo"
            style={{
              position: "absolute",
              width: halo.size,
              height: halo.size,
              borderRadius: halo.size / 2,
              backgroundColor: glow,
              opacity: halo.opacity,
              transform: [
                {
                  scale: held
                    ? swell.interpolate({ inputRange: [0, 1], outputRange: [0.92, 1.04] })
                    : 1,
                },
              ],
            }}
          />
        ))}
        <Animated.View
          testID="phone-mic-hold"
          accessible
          accessibilityRole="button"
          accessibilityLabel={label}
          accessibilityHint={held ? t("mobile.mic.slideCancel") : after}
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
            borderWidth: 1,
            borderColor: "rgba(255, 255, 255, 0.22)",
            alignItems: "center",
            justifyContent: "center",
            opacity: disabled ? 0.5 : 1,
            // A glow in the accent's own colour (board 03: 0 16 32, 45 %; held 0 10 28, 55 %).
            boxShadow: held ? `0 10px 28px ${rgba(glow, 0.55)}` : `0 16px 32px ${rgba(glow, 0.45)}`,
            transform: [{ scale }],
          }}>
          <Icon source={offButton ? "close" : "microphone"} size={44} color={ink} />
        </Animated.View>
      </View>
      {held && recording !== undefined && <Wave levels={levels} />}
      <Text
        testID="phone-mic-label"
        style={[styles.label, { color: offButton ? theme.voltip.danger : theme.colors.onSurface }]}
        numberOfLines={2}>
        {label}
      </Text>
      <View style={styles.hint}>
        {!held && (
          <Text testID="phone-mic-route" style={[styles.hintText, { color: theme.voltip.subtle }]}>
            {after}
          </Text>
        )}
        <Text style={[styles.hintText, { color: theme.voltip.subtle }]}>
          {t("mobile.mic.slideCancel")}
        </Text>
        {codec !== undefined && <Mono style={{ textAlign: "center" }}>{codec}</Mono>}
      </View>
      {children}
    </View>
  );
}

/** The take streams to a paired computer, which recognises and inserts the text. */
function ComputerTalk({ target }: { target: DeviceView }) {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { phone_take: take } = useUiState();
  const begin = useStart();
  const listening = take?.state.state === "listening";
  const now = useTicking(listening);
  const { frame, levels } = useTakeLevel(listening);
  const mine = take !== undefined && take.device === target.device.public_key ? take : undefined;
  const streaming = mine !== undefined && !phoneTakeFinal(mine.state);
  return (
    <TalkCard
      busy={take !== undefined && !phoneTakeFinal(take.state)}
      recording={
        mine?.state.state === "listening" ? formatElapsed(now - mine.started_at) : undefined
      }
      starting={mine?.state.state === "starting"}
      level={frame === undefined ? 0 : levelFraction(frame.rms_dbfs)}
      levels={levels}
      after={t("mobile.mic.afterComputer")}
      releaseLabel={t("mobile.mic.releaseTo", { name: target.device.name })}
      codec={streaming && mine.opus === true ? t("mobile.mic.codecOpus") : undefined}
      start={() =>
        begin(() => backend.invoke("phone_take_start", { publicKey: target.device.public_key }))
      }
      stop={() => void backend.invoke("phone_take_stop")}
      cancel={() => void backend.invoke("phone_take_cancel")}>
      {mine !== undefined &&
        mine.state.state !== "listening" &&
        mine.state.state !== "starting" && (
          <TakeState
            state={mine.state.state}
            tone={takeTone(mine.state)}
            line={phoneTakeLine(mine.state, now - mine.started_at, t)}
          />
        )}
    </TalkCard>
  );
}

/** The phone recognises the take itself (docs/dictation.md §20.7): the built-in service
 *  transcribes and polishes it, and the result lands on the phone's clipboard. */
function PhoneTalk() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { dictation } = useUiState();
  const begin = useStart();
  const phase = dictation.phase;
  const running = phase.phase === "listening" || phase.phase === "processing";
  const listening = phase.phase === "listening";
  const now = useTicking(listening);
  const { frame, levels } = useTakeLevel(listening);
  const elapsed = listening && phase.ready ? now - phase.started_at : 0;
  const line = localTakeLine(phase, elapsed, t);
  return (
    <TalkCard
      busy={running}
      recording={listening ? formatElapsed(elapsed) : undefined}
      starting={false}
      level={frame === undefined ? 0 : levelFraction(frame.rms_dbfs)}
      levels={levels}
      after={t("mobile.mic.afterPhone")}
      releaseLabel={t("mobile.mic.releaseLocal")}
      start={() => begin(() => backend.invoke("dictation_start"))}
      stop={() => void backend.invoke("dictation_stop")}
      cancel={() => void backend.invoke("dictation_cancel")}>
      {line !== "" && !listening && (
        <TakeState state={phase.phase} tone={localTone(phase)} line={line} />
      )}
    </TalkCard>
  );
}

/** The scene the phone's takes run with (docs/dictation.md §18; user decision 2026-10-01): the
 *  phone cannot tell which app the text goes to, so the user picks one, or none. A take sent to a
 *  computer runs with the computer's scene for its front app, so the chip is the phone's only. */
function ScenePicker({ disabled }: { disabled: boolean }) {
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const { scenes, settings } = useUiState();
  if (scenes.length === 0) return null;
  return (
    <SelectChip
      icon="creation-outline"
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

export type TakeRoute = "computer" | "phone";

/** Where takes go (board 03): to a computer when one is online and the user did not pick the
 *  phone, the phone's own recognition otherwise; a running take keeps its route. Above the card,
 *  the 识别方式 segment once a computer is paired, the computer picker when more than one is
 *  online, and the scene chip for the phone's own takes. */
export function PhoneMic({ desktops }: { desktops: readonly DeviceView[] }) {
  const { t } = useI18n();
  const { phone_take: take, dictation } = useUiState();
  const [choice, setChoice] = useState<TakeRoute>("computer");
  const [picked, setPicked] = useState<string | undefined>(undefined);
  const online = desktops.filter((d) => d.connection.state === "online");
  const toComputer = take !== undefined && !phoneTakeFinal(take.state);
  const onPhone = dictation.phase.phase === "listening" || dictation.phase.phase === "processing";
  const route: TakeRoute = toComputer
    ? "computer"
    : onPhone || online.length === 0
      ? "phone"
      : choice;
  // A running take stays with its computer even when that one went offline meanwhile.
  const target =
    (toComputer ? desktops.find((d) => d.device.public_key === take.device) : undefined) ??
    online.find((d) => d.device.public_key === picked) ??
    online[0];
  const running = toComputer || onPhone;
  return (
    <View style={{ gap: 12 }}>
      {desktops.length > 0 && (
        <Segmented<TakeRoute>
          testID="phone-route"
          label={t("mobile.mic.route")}
          value={route}
          disabled={running}
          onChange={setChoice}
          options={[
            {
              value: "computer",
              icon: "monitor",
              label:
                target === undefined
                  ? t("mobile.mic.toComputer")
                  : t("mobile.mic.toDesktop", { name: target.device.name }),
              disabled: online.length === 0,
              testID: "phone-route-computer",
            },
            {
              value: "phone",
              icon: "cellphone",
              label: t("mobile.mic.onPhone"),
              testID: "phone-route-phone",
            },
          ]}
        />
      )}
      {desktops.length > 0 && online.length === 0 && (
        <View testID="phone-mic-offline">
          <Hint>{t("mobile.mic.offline")}</Hint>
        </View>
      )}
      {route === "computer" && target !== undefined && online.length > 1 && (
        <SelectChip
          icon="monitor"
          label={t("mobile.mic.target")}
          value={target.device.public_key}
          disabled={running}
          options={online.map((d) => ({ value: d.device.public_key, label: d.device.name }))}
          onChange={setPicked}
          testID="phone-mic-target"
        />
      )}
      {route === "phone" && <ScenePicker disabled={running} />}
      {route === "computer" && target !== undefined ? (
        <ComputerTalk target={target} />
      ) : (
        <PhoneTalk />
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  card: { borderWidth: 1, paddingTop: 20, paddingBottom: 24, paddingHorizontal: 20, gap: 8 },
  eyebrow: { flexDirection: "row", alignItems: "center", gap: 8, minHeight: 18 },
  eyebrowText: { fontSize: 12, lineHeight: 16, fontWeight: "500" },
  stage: { height: STAGE, alignItems: "center", justifyContent: "center" },
  wave: {
    flexDirection: "row",
    alignItems: "center",
    alignSelf: "center",
    gap: 2,
    height: 28,
  },
  label: { fontSize: 17, lineHeight: 23, fontWeight: "500", textAlign: "center" },
  hint: { gap: 2, alignItems: "center" },
  hintText: { fontSize: 12, lineHeight: 18, textAlign: "center" },
});
