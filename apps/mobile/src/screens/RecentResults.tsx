import { type HistoryEntry, relativeTime } from "@voltip/shared";
import {
  Button,
  Card,
  Eyebrow,
  IconButton,
  useBackend,
  useI18n,
  useNow,
  useUiState,
} from "@voltip/ui";
import { TOUCH_ICON } from "../app/phone-ui";
import { useMobileShell } from "../app/shell";

/** How many of the phone's newest results the list shows. */
export const RECENT_SHOWN = 10;

/** Characters of a result the buttons' accessible names quote. */
const LABEL_CHARS = 24;

function quoted(text: string): string {
  const chars = Array.from(text.trim());
  return chars.length > LABEL_CHARS ? `${chars.slice(0, LABEL_CHARS).join("")}…` : chars.join("");
}

/** What the phone recognised itself and the takes it sent to a computer (docs/dictation.md §20.7;
 *  user decision 2026-10-03: the phone keeps those too), newest first, as the desktop's
 *  最近的结果: a label with the way to the whole history, then a hairline list. A row opens its
 *  entry's page, as a row of 记录 does (user report 2026-10-03), and copies or shares the result
 *  with the two buttons at its end. Nothing to show, no card. */
export function RecentResults() {
  const { backend } = useBackend();
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  const { history_recent: history } = useUiState();
  const now = useNow();
  const shown = history.filter((e) => e.text.trim().length > 0).slice(0, RECENT_SHOWN);
  if (shown.length === 0) return null;

  const copy = (entry: HistoryEntry) => {
    void backend.pasteText(entry.text).then(
      (outcome) => {
        if (outcome.kind === "failed") shell.toast(t("mobile.recent.copyFailed"), "danger");
        else shell.toast(t("mobile.recent.copied"));
      },
      () => {
        shell.toast(t("mobile.recent.copyFailed"), "danger");
      },
    );
  };
  const share = (entry: HistoryEntry) => {
    backend.invoke("phone_share_text", { text: entry.text }).catch((e: unknown) => {
      shell.toast(
        t("mobile.toast.error", { message: e instanceof Error ? e.message : String(e) }),
        "danger",
      );
    });
  };

  return (
    <section className="flex flex-col gap-2" data-testid="phone-recent">
      {/* The link keeps a 44 px target and lends the row none of its height; it is a link, so it
          takes the interface font, not the readout font of the slot it sits in. */}
      <Eyebrow
        className="pl-1"
        right={
          <Button
            variant="text"
            className="-my-3 h-11 font-ui text-[13px]"
            onClick={() => {
              shell.go("history");
            }}>
            {t("mobile.recent.all")}
          </Button>
        }>
        {t("mobile.recent.title")}
      </Eyebrow>
      <Card padding="none" className="overflow-hidden">
        <ul className="divide-y divide-border">
          {shown.map((entry) => (
            <li key={entry.id} className="relative" data-testid="phone-recent-row">
              {/* The row is the open target: this button covers it, the line at its foot included.
                  Copy and share sit over that line as buttons of their own, never inside it. */}
              <button
                type="button"
                className="block w-full px-4 pt-3 pb-11 text-left transition-colors hover:bg-inset active:bg-inset"
                aria-label={t("mobile.recent.openLabel", { text: quoted(entry.text) })}
                onClick={() => {
                  shell.go("entry", entry.id);
                }}>
                <span
                  className="line-clamp-3 text-[14px] leading-[22px] break-words whitespace-pre-wrap text-fg"
                  data-user-text>
                  {entry.text}
                </span>
              </button>
              <div className="pointer-events-none absolute inset-x-0 bottom-0 flex h-11 items-center pr-1 pl-4">
                <span className="mono flex-1 text-[11px] text-fg-subtle">
                  {relativeTime(Math.floor(entry.at_ms / 1000), now, locale)}
                </span>
                <IconButton
                  icon="copy"
                  size={28}
                  label={t("mobile.recent.copyLabel", { text: quoted(entry.text) })}
                  className={`${TOUCH_ICON} pointer-events-auto active:bg-inset2`}
                  onClick={() => {
                    copy(entry);
                  }}
                />
                <IconButton
                  icon="share"
                  size={28}
                  label={t("mobile.recent.shareLabel", { text: quoted(entry.text) })}
                  className={`${TOUCH_ICON} pointer-events-auto active:bg-inset2`}
                  onClick={() => {
                    share(entry);
                  }}
                />
              </div>
            </li>
          ))}
        </ul>
      </Card>
      <p className="px-1 text-[12px] leading-5 text-fg-subtle">{t("mobile.recent.body")}</p>
    </section>
  );
}
