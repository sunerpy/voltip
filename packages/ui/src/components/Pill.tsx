import { type TFunction, liveTextGap, zhT } from "@voltip/shared";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { Button } from "./Button";
import { Icon } from "./Icon";
import { Keycap, Keycaps } from "./Keycap";
import { Lamp } from "./Lamp";
import { Progress } from "./Progress";
import { Waveform } from "./Waveform";

export const PILL_STATES = [
  "armed",
  "listening",
  "locked",
  "processing",
  "inserted",
  "error",
  "cancel-armed",
  "blocked",
] as const;
export type PillState = (typeof PILL_STATES)[number];

/** The tones of the listening pill's live caption (docs/dictation.md §11–§12): sentences the
 *  streaming recogniser committed, the one still being spoken, and — under `live_inject` — the
 *  committed sentences already pasted into the front app (drawn fainter). */
export interface PillLiveCaption {
  committed: string;
  current: string;
  injected?: string;
}

/** How many characters (code points) of the live caption stay visible: the tail, so the words
 *  being spoken are always on screen. */
export const LIVE_CAPTION_MAX_CHARS = 40;

/** Keep the caption's tail: drop leading characters — the pasted text first, then the committed
 *  text — until at most `max` remain, marking the cut with a leading ellipsis on the first part
 *  that survives. Counted in code points (one CJK char = 1). */
export function clipLiveCaption(
  live: PillLiveCaption,
  max = LIVE_CAPTION_MAX_CHARS,
): PillLiveCaption & { clipped: boolean } {
  const injected = Array.from(live.injected ?? "");
  const committed = Array.from(live.committed);
  const current = Array.from(live.current);
  let drop = injected.length + committed.length + current.length - max;
  if (drop <= 0) return { ...live, clipped: false };
  let marked = false;
  const keep = (part: string[]): string => {
    if (!marked && drop >= part.length) {
      drop -= part.length;
      return "";
    }
    const kept = part.slice(drop).join("");
    drop = 0;
    if (marked) return kept;
    marked = true;
    return `…${kept}`;
  };
  const shownInjected = keep(injected);
  const shownCommitted = keep(committed);
  const shownCurrent = keep(current);
  return {
    ...(live.injected === undefined ? {} : { injected: shownInjected }),
    committed: shownCommitted,
    current: shownCurrent,
    clipped: true,
  };
}

/** The last `max` characters of a preview line, with a leading ellipsis when cut. */
export function clipTail(text: string, max = LIVE_CAPTION_MAX_CHARS): string {
  return clipLiveCaption({ committed: text, current: "" }, max).committed;
}

export interface PillProps {
  state: PillState;
  /** Overrides the default label for the state. */
  label?: string;
  /** Mono readout on the right (elapsed time, delivery method, bridge address). */
  readout?: string;
  /** `listening` only: the streaming preview drawn above the waveform — committed sentences in
   *  the normal colour, the current one dimmed, clipped to the tail (`LIVE_CAPTION_MAX_CHARS`). */
  live?: PillLiveCaption;
  /** `listening` only: the device has not delivered samples yet — the timer parks at 00:00 next to
   *  a "waiting for mic" hint instead of counting. */
  waiting?: boolean;
  /** `listening` only (docs/dictation.md §13): a short press locked the take — a lock mark replaces
   *  the lamp and says the next press stops. */
  locked?: boolean;
  /** `processing` only: the live preview's text, shown dimmed in place of the stage caption
   *  until the final text arrives. */
  preview?: string;
  /** 0..1 levels for the waveform states. */
  levels?: readonly number[];
  /** Mode tag: `本地` / `云端 openai` / `LLM`. */
  mode?: string;
  /** `listening` / `locked` / `processing`: the scene this take runs under (docs/dictation.md
   *  §18), a tag next to the mode tag; absent when no scene matched. */
  scene?: string;
  /** What the take does when it is not a plain dictation — `编辑` for a voice edit
   *  (docs/dictation.md §19): a leading tag in every state of a running or finished take. */
  tag?: string;
  /** `listening` / `locked`: what the take records — `麦克风` / `电脑声音` / `混合`
   *  (docs/dictation.md §22) — a tag right before the mode tag (source, then where it goes). */
  source?: string;
  /** `listening` / `locked`: how far a long take's recognition got (`已识别 12 段`,
   *  docs/dictation.md §22), after the timer. */
  progress?: string;
  /** The hotkey shown by the resting and blocked pills (`Ctrl Alt Space`); a running take always
   *  shows the Esc cancel hint instead. */
  keys?: string;
  /** Target app shown after `→` in the inserted state. */
  via?: string;
  onCopy?: () => void;
  onStop?: () => void;
  className?: string;
}

function perState(key: "label" | "caption"): Readonly<Record<PillState, string>> {
  const text = (state: PillState) => zhT.t(`ui.pill.${key}.${state}`);
  return {
    armed: text("armed"),
    listening: text("listening"),
    locked: text("locked"),
    processing: text("processing"),
    inserted: text("inserted"),
    error: text("error"),
    "cancel-armed": text("cancel-armed"),
    blocked: text("blocked"),
  };
}

/** Default label of each state in the default locale (zh-CN); `Pill` itself reads `useT()`. */
export const PILL_DEFAULT_LABEL: Readonly<Record<PillState, string>> = perState("label");

/** Spec-sheet captions in the default locale; `pillCaption` localizes them. */
export const PILL_CAPTIONS: Readonly<Record<PillState, string>> = perState("caption");

export function pillCaption(state: PillState, t: TFunction): string {
  return t(`ui.pill.caption.${state}`);
}

function ModeTag({ children }: { children: string }) {
  return (
    <span className="mono rounded-6 bg-inset px-1.5 py-0.5 text-[10px] text-fg-muted">
      {children}
    </span>
  );
}

/** What the take records, in the mode tag's style. */
function SourceTag({ name }: { name: string | undefined }) {
  if (name === undefined || name.length === 0) return null;
  return (
    <span
      className="mono shrink-0 rounded-6 bg-inset px-1.5 py-0.5 text-[10px] text-fg-muted"
      data-testid="pill-source">
      {name}
    </span>
  );
}

/** A long take's recognition count next to the timer. */
function SegmentCount({ text }: { text: string | undefined }) {
  if (text === undefined || text.length === 0) return null;
  return (
    <span className="shrink-0 text-[11px] text-pill-muted" data-testid="pill-progress">
      {text}
    </span>
  );
}

/** The matched scene's name (the user's text, clipped), titled `场景：<name>`. */
function SceneTag({ name }: { name: string | undefined }) {
  const t = useT();
  if (name === undefined || name.length === 0) return null;
  return (
    <span
      className="max-w-[120px] truncate rounded-6 border border-pill-border px-1.5 py-0.5 text-[10px] text-pill-muted"
      data-testid="pill-scene"
      title={t("ui.pill.scene", { name })}>
      {name}
    </span>
  );
}

/** The cancel hint of a running take: Esc outlined in the danger colour and the word for what it
 *  does (user feedback 2026-09-29: the bare Esc keycap did not say it cancels). */
function EscCancel() {
  const t = useT();
  return (
    <span
      role="img"
      aria-label={t("ui.pill.escCancelLabel")}
      title={t("ui.pill.escCancelLabel")}
      className="inline-flex shrink-0 items-center gap-1 text-[11px] text-danger"
      data-testid="pill-esc-cancel">
      <Keycap tone="danger">Esc</Keycap>
      <span aria-hidden>{t("ui.pill.escCancel")}</span>
    </span>
  );
}

/** The states a take's kind tag shows in (not the resting, blocked or bridge capsules). */
const TAGGED_STATES: ReadonlySet<PillState> = new Set<PillState>([
  "listening",
  "locked",
  "processing",
  "inserted",
  "error",
  "cancel-armed",
]);

/** The take's kind (`编辑`), leading the capsule in the accent colour. */
function KindTag({ label }: { label: string }) {
  return (
    <span
      className="shrink-0 rounded-6 bg-accent px-1.5 py-0.5 text-[10px] font-semibold text-accent-fg"
      data-testid="pill-tag">
      {label}
    </span>
  );
}

/** The pill never grows past the overlay window (480 px, `overlay.rs` `OVERLAY_WIDTH`) less its
 *  margins; when a scene tag, the waiting hint and the Esc hint all show, the waveform gives way
 *  and loses its oldest bars (the newest stay, at the right end). */
const YIELDING_WAVE = "min-w-0 shrink justify-end overflow-hidden";

/** The overlay capsule (height 40, radius 999). Never focusable: it must not steal the target window. */
export function Pill({
  state,
  label,
  readout,
  live,
  waiting = false,
  locked = false,
  preview,
  levels = [],
  mode,
  scene,
  tag,
  source,
  progress,
  keys,
  via,
  onCopy,
  onStop,
  className,
}: PillProps) {
  const t = useT();
  const text = label ?? t(`ui.pill.label.${state}`);
  const local = mode ?? t("ui.pill.local");
  const danger = state === "error" || state === "cancel-armed" || state === "blocked";
  const accentRing = state === "inserted";
  // The live caption sits above the waveform row: the capsule grows to two rows (56 px, the
  // overlay window's 64 px minus its 8 px top inset) and shrinks back once the preview is gone.
  const caption = state === "listening" && live !== undefined ? clipLiveCaption(live) : undefined;
  const listeningRow = (
    <>
      {locked ? (
        <span
          role="img"
          aria-label={t("ui.pill.lockedHint")}
          title={t("ui.pill.lockedHint")}
          data-testid="pill-lock"
          className="flex shrink-0 items-center text-pill-fg">
          <Icon name="lock" size={13} />
        </span>
      ) : (
        <Lamp tone="accent" />
      )}
      <Waveform levels={levels} height={16} bars={36} className={YIELDING_WAVE} />
      <span className="mono text-[11px] text-pill-muted">
        {waiting ? "00:00" : (readout ?? "00:00")}
      </span>
      {waiting && (
        <span className="text-[11px] text-pill-muted" data-testid="pill-waiting">
          {t("ui.pill.waitingMic")}
        </span>
      )}
      <SegmentCount text={progress} />
      <SourceTag name={source} />
      <ModeTag>{local}</ModeTag>
      <SceneTag name={scene} />
      <EscCancel />
    </>
  );
  return (
    <div
      role="status"
      aria-live="polite"
      data-state={state}
      tabIndex={-1}
      className={cx(
        "inline-flex min-w-[52px] max-w-[464px] items-center gap-2.5 rounded-pill border bg-pill-bg pr-3.5 pl-3.5 text-[13px] font-medium whitespace-nowrap text-pill-fg shadow-pill select-none",
        caption ? "h-14" : "h-10",
        danger ? "border-danger" : accentRing ? "border-accent" : "border-pill-border",
        className,
      )}>
      {tag !== undefined && tag.length > 0 && TAGGED_STATES.has(state) && <KindTag label={tag} />}
      {state === "armed" && (
        // the resting pill is the 52 px three-dot capsule; the hotkey lives in its tooltip
        // and accessible name, not in the pill.
        <span
          className="inline-flex items-center gap-1"
          role="img"
          aria-label={`${text} · ${keys ?? "Ctrl Alt Space"} · ${local}`}
          title={`${text} · ${keys ?? "Ctrl Alt Space"} · ${local}`}>
          <span className="h-1 w-1 rounded-full bg-pill-muted" />
          <span className="h-1 w-1 rounded-full bg-pill-muted" />
          <span className="h-1 w-1 rounded-full bg-pill-muted" />
        </span>
      )}
      {state === "listening" &&
        (caption ? (
          <span className="flex min-w-0 flex-col gap-1">
            <span
              className="flex items-center gap-1.5 text-[12px] leading-4"
              data-testid="pill-live"
              data-clipped={caption.clipped ? "true" : undefined}>
              {/* End-aligned inside a clipped box: when 40 CJK characters are wider than the
                  window, the *start* of the line is what gets cut, so the words being spoken stay
                  on screen (a plain `truncate` would hide the tail instead). */}
              <span className="flex min-w-0 max-w-[400px] justify-end overflow-hidden">
                <span className="shrink-0 whitespace-nowrap">
                  {caption.injected !== undefined && caption.injected.length > 0 && (
                    <span
                      className="text-pill-muted opacity-60"
                      data-testid="pill-live-injected"
                      title={t("ui.pill.injectedTitle")}>
                      {caption.injected}
                    </span>
                  )}
                  <span data-testid="pill-live-committed">
                    {liveTextGap(caption.injected ?? "", caption.committed)}
                    {caption.committed}
                  </span>
                  {caption.current.length > 0 && (
                    <span className="text-pill-muted" data-testid="pill-live-current">
                      {liveTextGap(
                        `${caption.injected ?? ""}${caption.committed}`,
                        caption.current,
                      )}
                      {caption.current}
                    </span>
                  )}
                </span>
              </span>
              <span className="mono rounded-6 bg-inset px-1.5 py-0.5 text-[10px] text-fg-subtle">
                {t("ui.pill.livePreview")}
              </span>
            </span>
            <span className="flex items-center gap-2.5">{listeningRow}</span>
          </span>
        ) : (
          listeningRow
        ))}
      {state === "locked" && (
        <>
          <Icon name="lock" size={13} className="text-pill-muted" />
          <Waveform levels={levels} height={16} bars={36} className={YIELDING_WAVE} />
          <span className="mono text-[11px] text-pill-muted">{readout ?? "00:00"}</span>
          <SegmentCount text={progress} />
          <SourceTag name={source} />
          <ModeTag>{local}</ModeTag>
          <SceneTag name={scene} />
          <EscCancel />
          <button
            type="button"
            aria-label={t("ui.pill.stop")}
            onClick={onStop}
            className="flex h-6 w-6 items-center justify-center rounded-full bg-inset text-fg hover:bg-inset2">
            <Icon name="stop" size={10} />
          </button>
        </>
      )}
      {state === "processing" && (
        <>
          <Lamp tone="accent" pulse />
          <Waveform
            levels={levels}
            state="frozen"
            height={14}
            bars={20}
            className={YIELDING_WAVE}
          />
          <span className="flex min-w-0 flex-col gap-1">
            {preview !== undefined && preview.length > 0 ? (
              <span className="max-w-[300px] truncate text-pill-muted" data-testid="pill-preview">
                {clipTail(preview)}
              </span>
            ) : (
              <span>{text}</span>
            )}
            <Progress indeterminate size={2} tone="ink" className="w-[64px]" />
          </span>
          <ModeTag>{local}</ModeTag>
          <SceneTag name={scene} />
          {/* The current step's time; nothing when the caller has none to give (no fixed 0.0 s). */}
          {readout !== undefined && (
            <span className="mono text-[11px] text-pill-muted" data-testid="pill-stage-time">
              {readout}
            </span>
          )}
          <EscCancel />
        </>
      )}
      {state === "inserted" && (
        <>
          <span className="flex h-4 w-4 items-center justify-center rounded-full bg-accent text-accent-fg">
            <Icon name="check" size={10} strokeWidth={3} />
          </span>
          <span>{text}</span>
          <Waveform levels={levels} state="collapsed" height={8} bars={12} />
          {via && <span className="mono text-[11px] text-pill-muted">→ {via}</span>}
          {readout && <span className="mono text-[11px] text-pill-muted">{readout}</span>}
        </>
      )}
      {state === "error" && (
        <>
          <Icon name="alert" size={14} className="text-danger" />
          <span>{text}</span>
          {/* Only when there is something to copy: a silent take has no text to offer. */}
          {onCopy && (
            <Button size="sm" variant="primary" onClick={onCopy} className="h-6">
              {t("ui.pill.copyText")}
            </Button>
          )}
        </>
      )}
      {state === "cancel-armed" && (
        <>
          <Lamp tone="danger" />
          <Waveform levels={levels} tone="danger" height={16} bars={28} />
          <span>{text}</span>
          {readout && <span className="mono text-[11px] text-pill-muted">{readout}</span>}
        </>
      )}
      {state === "blocked" && (
        <>
          <span
            className="inline-block h-2 w-2 rounded-full border-[1.5px] border-danger"
            aria-hidden
          />
          <span>{text}</span>
          <Keycaps keys={keys ?? "Ctrl Alt Space"} />
        </>
      )}
    </div>
  );
}

export interface LiveCaptionProps {
  committed: string;
  tail: string;
  tier: "preview" | "final" | "failed";
  elapsed: string;
  engine: string;
  queueDepth?: number;
  levels?: readonly number[];
  className?: string;
}

/** 420×88 streaming caption replacing the pill when partial results are available. */
export function LiveCaption({
  committed,
  tail,
  tier,
  elapsed,
  engine,
  queueDepth = 0,
  levels = [],
  className,
}: LiveCaptionProps) {
  const t = useT();
  const empty = committed.length === 0 && tail.length === 0;
  return (
    <div
      role="status"
      aria-live="polite"
      data-tier={tier}
      className={cx(
        "flex w-[420px] flex-col gap-2 rounded-14 bg-pill-bg p-3.5 text-pill-fg hairline shadow-pill",
        className,
      )}>
      <div className="flex items-center gap-2">
        <Lamp tone={tier === "failed" ? "danger" : "accent"} />
        <span className="text-[13px] font-medium">
          {tier === "failed" ? t("ui.pill.caption_failed") : t("ui.pill.caption_listening")}
        </span>
        <span
          className={cx(
            "mono rounded-6 px-1.5 py-0.5 text-[10px]",
            tier === "final" ? "bg-accent-soft text-accent-text" : "bg-inset text-fg-subtle",
          )}>
          {tier === "final" ? t("ui.pill.tier.final") : t("ui.pill.tier.preview")}
        </span>
        <Waveform levels={levels} height={10} bars={16} className="ml-auto" />
        <span className="mono text-[11px] text-pill-muted">{elapsed}</span>
      </div>
      <p className="h-10 overflow-hidden text-[14px] leading-5">
        {empty ? (
          <span className="text-fg-subtle">……</span>
        ) : (
          <>
            <span className={tier === "failed" ? "text-fg-subtle" : "text-fg"}>{committed}</span>
            {tail.length > 0 && <span className="text-fg-subtle"> {tail}</span>}
          </>
        )}
      </p>
      <div className="mono flex items-center justify-between text-[10px] text-fg-subtle">
        <span>{engine}</span>
        {queueDepth > 0 && <span>{t("ui.pill.queued", { n: queueDepth })}</span>}
      </div>
    </div>
  );
}
