import {
  type ServiceKind,
  type ServiceUsage,
  audioAmount,
  othersToday,
  resetClock,
  serviceUsage,
  tighter,
  usageAmount,
  usageValue,
  usesBuiltin,
} from "@voltip/shared";
import { Icon, USAGE_TEXT_TONE, UsageBar, cx, useI18n, useUiState, useUsageNow } from "@voltip/ui";

/** The sidebar's 今日用量 card (Penpot 「用量与额度」 board 14, option A): the built-in service's
 *  day for recognition and the clean-up — bars against the limits, amber from 80 %, red at the
 *  limit — or this device's own count while neither uses it. Its full form shows when the sidebar
 *  has the height for it (`USAGE_FULL_HEIGHT`, a container query on the sidebar), one line
 *  otherwise (the default 720 px window at the default 字号); the icon rail shows two rings with
 *  the numbers in the tooltip. A click opens 用量. */
export function SidebarUsage({
  collapsed,
  active = false,
  disabled,
  onOpen,
}: {
  collapsed: boolean;
  /** The 用量 page is showing. */
  active?: boolean;
  disabled: boolean;
  onOpen: () => void;
}) {
  const card = useUsageCard();
  if (card === undefined) return null;
  const props = { card, disabled, onOpen, active };
  if (collapsed) return <RailUsage {...props} />;
  return (
    <>
      <FullUsage {...props} />
      <CompactUsage {...props} />
    </>
  );
}

interface PartProps {
  card: UsageCard;
  active: boolean;
  disabled: boolean;
  onOpen: () => void;
}

/** One service's row: the built-in service's day, or this device's count of another provider. */
interface Row {
  kind: ServiceKind;
  label: string;
  value: string;
  usage: ServiceUsage;
  builtin: boolean;
}

interface UsageCard {
  rows: Row[];
  /** What colours the icon and fills the one-line form: the tighter built-in service. */
  worst?: ServiceUsage;
  /** The header's right side: when the day resets, 暂不限量, 本机统计 or 无法获取. */
  right: string;
  /** Neither service uses the built-in one. */
  own: boolean;
  unknown: boolean;
  unlimited: boolean;
  /** The service has not reported yet (the first minutes after start). */
  pending: boolean;
}

/** The card's content from `UiState.usage` and the engines in use; nothing in a build without the
 *  built-in service. */
function useUsageCard(): UsageCard | undefined {
  const { t, locale } = useI18n();
  const { usage: view, engines } = useUiState();
  const now = useUsageNow();
  if (!view.builtin) return undefined;
  const rows: Row[] = (["asr", "llm"] as const).map((kind) => {
    const usage = serviceUsage(view, kind, now);
    const builtin = usesBuiltin(engines, kind);
    if (builtin)
      return {
        kind,
        label: t(`usage.short.${kind}`),
        value: usageValue(usage, locale),
        usage,
        builtin,
      };
    const other = othersToday(view, kind);
    const amount =
      kind === "asr" ? audioAmount(other.amount, locale) : usageAmount("llm", other.amount, locale);
    const value =
      other.provider === undefined
        ? amount
        : t("usage.card.ownValue", { amount, provider: t(`engines.provider.${other.provider}`) });
    return { kind, label: t(`usage.short.${kind}`), value, usage, builtin };
  });
  const builtinRows = rows.filter((r) => r.builtin);
  const own = builtinRows.length === 0;
  const worst =
    builtinRows.length === 0
      ? undefined
      : builtinRows.map((r) => r.usage).reduce((a, b) => tighter(a, b));
  const unknown = builtinRows.some((r) => r.usage.tone === "unknown");
  const pending = builtinRows.length > 0 && builtinRows.every((r) => r.usage.tone === "pending");
  const unlimited =
    builtinRows.length > 0 && builtinRows.every((r) => r.usage.tone === "unlimited");
  const reset = builtinRows.find((r) => r.usage.resetAtMs !== undefined)?.usage.resetAtMs;
  const right = own
    ? t("usage.card.own")
    : unknown
      ? t("usage.unavailable")
      : unlimited
        ? t("usage.unlimited")
        : reset === undefined
          ? ""
          : t("usage.resetShort", { time: resetClock(reset) });
  return {
    rows,
    ...(worst === undefined ? {} : { worst }),
    right,
    own,
    unknown,
    unlimited,
    pending,
  };
}

const CARD_CLASS =
  "w-full rounded-row border bg-field text-left transition-colors hover:border-fg-subtle disabled:cursor-not-allowed disabled:opacity-40";

/** The card's border: the accent while the 用量 page shows. */
function edge(active: boolean): string {
  return active ? "border-accent/60" : "border-field-edge";
}

/** The sidebar's content height (inside its padding) from which the full card shows: what the
 *  brand row, the seven nav entries, the status card, this card and the footer take together —
 *  their boxes in rem (the 字号 setting scales them), their lines of text in px — with about 12 px
 *  to spare (measured at 14 px: 692 px of content without a scrolling nav). Below it the one-line
 *  form shows; a webview that cannot read the query shows the one-line form. */
export const USAGE_FULL_HEIGHT = "calc(38rem + 160px)";

/** Shown at `USAGE_FULL_HEIGHT` and up (a container query on the sidebar's `nav`). */
const FULL_ONLY = "hidden [@container_sidebar_(min-height:calc(38rem_+_160px))]:flex";
const COMPACT_ONLY = "flex [@container_sidebar_(min-height:calc(38rem_+_160px))]:hidden";

function gaugeTone(card: UsageCard): string {
  const tone = card.worst?.tone;
  return tone === "out" ? "text-danger" : tone === "near" ? "text-warning" : "text-accent";
}

function FullUsage({ card, active, disabled, onOpen }: PartProps) {
  const { t } = useI18n();
  return (
    <button
      type="button"
      data-testid="sidebar-usage-card"
      data-mode={
        card.own ? "own" : card.unknown ? "unknown" : card.unlimited ? "unlimited" : "builtin"
      }
      data-tone={card.worst?.tone}
      title={t("usage.card.open")}
      disabled={disabled}
      onClick={onOpen}
      aria-current={active ? "page" : undefined}
      className={cx(CARD_CLASS, edge(active), FULL_ONLY, "mt-3 flex-col gap-2 px-3.5 py-3")}>
      <span className="flex items-center gap-1.5">
        <Icon name="gauge" size={14} className={cx("shrink-0", gaugeTone(card))} />
        <span className="text-[12px] font-medium text-fg">{t("usage.title")}</span>
        <span className="ml-auto truncate text-[11px] text-fg-subtle">{card.right}</span>
        <Icon name="chevronRight" size={12} className="shrink-0 text-fg-subtle" />
      </span>
      {card.unknown ? (
        <span className="flex flex-col gap-1">
          <span className="text-[12px] text-fg-muted">{t("usage.unavailableBody")}</span>
          <span className="text-[11px] text-fg-subtle">{t("usage.unavailableHint")}</span>
        </span>
      ) : card.pending ? (
        <span className="text-[12px] text-fg-subtle">{t("usage.pending")}</span>
      ) : (
        <span className="flex flex-col gap-2">
          {card.rows.map((row) => (
            <span
              key={row.kind}
              className="flex flex-col gap-1.5"
              data-testid={`sidebar-usage-${row.kind}`}>
              <span className="flex min-w-0 items-center gap-2 text-[12px]">
                <span className="shrink-0 text-fg-muted">{row.label}</span>
                <span
                  className={cx(
                    "ml-auto truncate font-medium",
                    row.builtin && (row.usage.tone === "near" || row.usage.tone === "out")
                      ? USAGE_TEXT_TONE[row.usage.tone]
                      : "text-fg",
                  )}>
                  {row.value}
                </span>
              </span>
              {row.builtin &&
                (row.usage.tone === "ok" ||
                  row.usage.tone === "near" ||
                  row.usage.tone === "out") && (
                  <UsageBar
                    share={row.usage.tone === "out" ? 1 : row.usage.share}
                    tone={row.usage.tone}
                    size={6}
                    label={`${t(`usage.service.${row.kind}`)} · ${row.value}`}
                  />
                )}
            </span>
          ))}
          {(card.own || card.unlimited) && (
            <span className="border-t border-border pt-1.5 text-[11px] text-fg-subtle">
              {card.own ? t("usage.card.ownNote") : t("usage.unlimitedShort")}
            </span>
          )}
        </span>
      )}
    </button>
  );
}

/** The card as one line, for a sidebar without the height for the full one. */
function CompactUsage({ card, active, disabled, onOpen }: PartProps) {
  const { t, locale } = useI18n();
  const worst = card.worst;
  const value = card.own
    ? t("usage.card.own")
    : card.pending
      ? t("usage.pending")
      : card.unknown
        ? t("usage.unavailable")
        : card.unlimited
          ? t("usage.unlimited")
          : worst === undefined
            ? ""
            : t(worst.tone === "ok" ? "usage.card.compactNumbers" : "usage.card.compactValue", {
                service: t(`usage.short.${worst.kind}`),
                value: usageValue(worst, locale),
              });
  return (
    <button
      type="button"
      data-testid="sidebar-usage-compact"
      title={t("usage.card.open")}
      disabled={disabled}
      onClick={onOpen}
      aria-current={active ? "page" : undefined}
      className={cx(CARD_CLASS, edge(active), COMPACT_ONLY, "mt-2 h-9 items-center gap-1.5 px-3")}>
      <Icon name="gauge" size={14} className={cx("shrink-0", gaugeTone(card))} />
      <span
        className={cx(
          "min-w-0 flex-1 truncate text-[12px] font-medium",
          worst !== undefined && (worst.tone === "near" || worst.tone === "out")
            ? USAGE_TEXT_TONE[worst.tone]
            : "text-fg",
        )}>
        {t("usage.card.compact", { value })}
      </span>
      <Icon name="chevronRight" size={12} className="shrink-0 text-fg-subtle" />
    </button>
  );
}

/** Two rings for the icon rail, recognition outside and the clean-up inside; the tooltip and the
 *  button's name carry the numbers. */
function RailUsage({ card, active, disabled, onOpen }: PartProps) {
  const { t } = useI18n();
  const [asr, llm] = card.rows;
  const label =
    asr === undefined || llm === undefined
      ? t("usage.title")
      : t("usage.card.rail", { asr: asr.value, llm: llm.value, reset: card.right });
  return (
    <button
      type="button"
      data-testid="sidebar-usage-rail"
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onOpen}
      aria-current={active ? "page" : undefined}
      className={cx(CARD_CLASS, edge(active), "mt-3 flex h-11 items-center justify-center")}>
      <svg aria-hidden width="28" height="28" viewBox="0 0 28 28">
        {asr !== undefined && <Ring r={12} usage={asr.builtin ? asr.usage : undefined} />}
        {llm !== undefined && <Ring r={7} usage={llm.builtin ? llm.usage : undefined} />}
      </svg>
    </button>
  );
}

const RING_STROKE: Readonly<Record<string, string>> = {
  ok: "stroke-accent",
  near: "stroke-warning-fill",
  out: "stroke-danger",
};

function Ring({ r, usage }: { r: number; usage: ServiceUsage | undefined }) {
  const length = 2 * Math.PI * r;
  const share = usage === undefined ? 0 : usage.tone === "out" ? 1 : usage.share;
  const stroke = usage === undefined ? undefined : RING_STROKE[usage.tone];
  return (
    <>
      <circle cx="14" cy="14" r={r} fill="none" strokeWidth="3" className="stroke-led-off" />
      {stroke !== undefined && share > 0 && (
        <circle
          cx="14"
          cy="14"
          r={r}
          fill="none"
          strokeWidth="3"
          strokeLinecap="round"
          strokeDasharray={`${(share * length).toFixed(2)} ${length.toFixed(2)}`}
          transform="rotate(-90 14 14)"
          className={stroke}
        />
      )}
    </>
  );
}
