/** A message to the owner for every new report (docs/feedback.md): a Telegram bot posts it to one
 *  chat. It carries the start of the message and what the report says about the app, never the
 *  contact or the attachments themselves; those stay in the database and on the admin page. The
 *  report is stored first, and a message that fails to go out changes nothing about it. */
import type { Diagnostics, Kind } from "./validate";

/** What the message says about one stored report. */
export interface Notice {
  id: string;
  kind: Kind;
  message: string;
  /** Whether the user left a way to reach them; the value itself stays out of the message. */
  contact: boolean;
  diagnostics: Diagnostics;
  attachments: number;
  country: string | null;
}

export type Notifier = (notice: Notice) => Promise<void>;

/** The start of the report's message the notice quotes, in UTF-16 code units. */
export const NOTICE_MESSAGE_CHARS = 600;
export const TELEGRAM_API = "https://api.telegram.org";
export const NOTIFY_TIMEOUT_MS = 10_000;

const KIND_LABEL: Record<Kind, string> = { bug: "问题", idea: "建议", other: "其他" };

/** The notice as plain text: Telegram shows a bare URL as a link, and plain text needs no escaping
 *  of what the user wrote. */
export function noticeText(notice: Notice, adminUrl?: string): string {
  const message = notice.message.trim();
  const quoted =
    message.length > NOTICE_MESSAGE_CHARS
      ? `${message.slice(0, NOTICE_MESSAGE_CHARS).trimEnd()}…`
      : message;
  const d = notice.diagnostics;
  const platform = [d.os, d.arch].filter(Boolean).join(" ");
  const about = [d.app_version, platform, d.locale, notice.country].filter(Boolean).join(" · ");
  const extras = [
    notice.attachments > 0 ? `附件 ${notice.attachments} 个` : "",
    notice.contact ? "留了联系方式" : "",
  ].filter(Boolean);
  return [
    `Voltip 新反馈 · ${KIND_LABEL[notice.kind]}`,
    quoted,
    "",
    about,
    extras.join(" · "),
    adminUrl ?? "",
    `id ${notice.id}`,
  ]
    .filter((line, i) => i === 2 || line.length > 0)
    .join("\n");
}

export interface NotifyEnv {
  /** The bot that posts the notice; with the chat unset as well, nothing is sent. */
  TELEGRAM_BOT_TOKEN?: string;
  /** The chat the bot posts to: the owner's, after they sent the bot /start. */
  TELEGRAM_CHAT_ID?: string;
  /** The admin page the notice links to. */
  ADMIN_URL?: string;
}

/** The Telegram notifier, or `undefined` when the bot or the chat is not configured. Errors carry
 *  the status only: the request URL holds the bot token. */
export function telegramNotifier(
  env: NotifyEnv,
  send: typeof fetch = (input, init) => fetch(input, init),
): Notifier | undefined {
  const token = env.TELEGRAM_BOT_TOKEN ?? "";
  const chat = env.TELEGRAM_CHAT_ID ?? "";
  if (token.length === 0 || chat.length === 0) return undefined;
  return async (notice) => {
    const answer = await send(`${TELEGRAM_API}/bot${token}/sendMessage`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        chat_id: chat,
        text: noticeText(notice, env.ADMIN_URL || undefined),
        disable_web_page_preview: true,
      }),
      signal: AbortSignal.timeout(NOTIFY_TIMEOUT_MS),
    });
    if (!answer.ok) throw new Error(`telegram answered ${answer.status}`);
  };
}
