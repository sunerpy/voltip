import { type HistoryEntry, relativeTime } from "@voltip/shared";
import { Button, Card, useBackend, useI18n, useNow, useUiState } from "@voltip/ui";
import { useMobileShell } from "../app/shell";

/** How many of the phone's newest results the list shows. */
export const RECENT_SHOWN = 10;

/** Characters of a result the buttons' accessible names quote. */
const LABEL_CHARS = 24;

function quoted(text: string): string {
  const chars = Array.from(text.trim());
  return chars.length > LABEL_CHARS ? `${chars.slice(0, LABEL_CHARS).join("")}…` : chars.join("");
}

/** What the phone recognised itself (docs/dictation.md §20.7), newest first: each result opens its
 *  entry's page, as a row of 记录 does (user report 2026-10-03), and can be copied again or handed
 *  to another app with the buttons beside it. A take streamed to a computer is in that computer's
 *  history, not here; nothing to show, no card. */
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
    <Card className="flex flex-col gap-3" data-testid="phone-recent">
      <div className="flex flex-col gap-1">
        <span className="flex items-center gap-2">
          <span className="flex-1 text-[15px] font-semibold text-fg">
            {t("mobile.recent.title")}
          </span>
          <Button
            size="sm"
            variant="text"
            onClick={() => {
              shell.go("history");
            }}>
            {t("mobile.recent.all")}
          </Button>
        </span>
        <p className="text-[12px] text-fg-muted">{t("mobile.recent.body")}</p>
      </div>
      <ul className="flex flex-col gap-2">
        {shown.map((entry) => (
          <li
            key={entry.id}
            className="flex flex-col gap-2 rounded-10 bg-inset p-3"
            data-testid="phone-recent-row">
            {/* The text opens the entry; copy and share below are buttons of their own, never
                inside this one. */}
            <button
              type="button"
              className="rounded-6 text-left"
              aria-label={t("mobile.recent.openLabel", { text: quoted(entry.text) })}
              onClick={() => {
                shell.go("entry", entry.id);
              }}>
              <span
                className="line-clamp-4 whitespace-pre-wrap break-words text-[14px] leading-6 text-fg"
                data-user-text>
                {entry.text}
              </span>
            </button>
            <div className="flex items-center gap-2">
              <span className="text-[12px] text-fg-subtle">
                {relativeTime(Math.floor(entry.at_ms / 1000), now, locale)}
              </span>
              <Button
                size="sm"
                variant="ghost"
                icon="copy"
                className="ml-auto"
                aria-label={t("mobile.recent.copyLabel", { text: quoted(entry.text) })}
                onClick={() => {
                  copy(entry);
                }}>
                {t("mobile.recent.copy")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                icon="share"
                aria-label={t("mobile.recent.shareLabel", { text: quoted(entry.text) })}
                onClick={() => {
                  share(entry);
                }}>
                {t("mobile.recent.share")}
              </Button>
            </div>
          </li>
        ))}
      </ul>
    </Card>
  );
}
