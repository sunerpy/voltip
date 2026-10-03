import {
  type DeviceView,
  MAX_PHONE_TEXT_CHARS,
  type PhoneTextSource,
  type SentText,
  type TFunction,
  coreMessageText,
  formatDateTime,
  sentTextFinal,
} from "@voltip/shared";
import { Button, Panel, Select, Textarea, cx, useBackend, useI18n, useUiState } from "@voltip/ui";
import { useState } from "react";
import { StateLine, TOUCH, TOUCH_TEXTAREA } from "../app/phone-ui";
import { useMobileShell } from "../app/shell";

/** Characters as the core counts them (code points), not UTF-16 units. */
function chars(text: string): number {
  return Array.from(text).length;
}

/** The line under a sent text: where it is on the desktop. */
export function sentTextLine(text: SentText, t: TFunction): string {
  switch (text.state.state) {
    case "sending":
      return t("mobile.send.state.sending");
    case "queued":
      return t("mobile.send.state.queued", { name: text.device_name });
    case "delivered":
      return t(text.state.pasted ? "mobile.send.state.pasted" : "mobile.send.state.clipboard", {
        name: text.device_name,
      });
    case "failed":
      return t(`mobile.send.state.failed.${text.state.code}`, { message: text.state.message });
  }
}

function tone(text: SentText): "ok" | "accent" | "danger" {
  if (text.state.state === "failed") return "danger";
  return sentTextFinal(text.state) ? "ok" : "accent";
}

/** The phone as the desktop's keyboard (docs/dictation.md §20.6): type (or paste) a text and send
 *  it, or send the clipboard as it is; the desktop inserts it at its cursor, after its own take
 *  when one is running. The list below keeps what was sent, newest first, with the desktop's
 *  answer. Nothing shows until a paired desktop is online. */
export function SendText({ desktops }: { desktops: readonly DeviceView[] }) {
  const { backend } = useBackend();
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  const { sent_texts: sent } = useUiState();
  const online = desktops.filter((d) => d.connection.state === "online");
  const [picked, setPicked] = useState<string | undefined>(undefined);
  const target = online.find((d) => d.device.public_key === picked) ?? online[0];
  const [draft, setDraft] = useState("");
  const count = chars(draft);
  const tooLong = count > MAX_PHONE_TEXT_CHARS;
  const failed = (e: unknown) => {
    shell.toast(
      t("mobile.toast.error", {
        message: coreMessageText(e instanceof Error ? e.message : String(e)),
      }),
      "danger",
    );
  };
  const send = (body: string, source: PhoneTextSource) => {
    if (target === undefined) return Promise.resolve();
    return backend.invoke("phone_text_send", { publicKey: target.device.public_key, body, source });
  };
  const sendDraft = async () => {
    try {
      await send(draft, "typed");
      setDraft("");
    } catch (e) {
      failed(e);
    }
  };
  const sendClipboard = async () => {
    try {
      const text = await backend.phoneClipboardRead();
      if (text === null || text.trim().length === 0) {
        shell.toast(t("mobile.send.clipboardEmpty"));
        return;
      }
      await send(text, "clipboard");
    } catch (e) {
      failed(e);
    }
  };

  return (
    <Panel
      eyebrow={t("mobile.send.title")}
      bodyClassName="flex flex-col gap-3"
      data-testid="send-text">
      <p className="text-[12px] leading-5 text-fg-muted">
        {target === undefined ? t("mobile.send.noDesktop") : t("mobile.send.body")}
      </p>
      {target !== undefined && (
        <>
          {online.length > 1 && (
            <Select
              label={t("mobile.send.target")}
              value={target.device.public_key}
              options={online.map((d) => ({ value: d.device.public_key, label: d.device.name }))}
              onChange={setPicked}
              data-user-text=""
            />
          )}
          <div className="flex flex-col gap-1">
            <Textarea
              className={TOUCH_TEXTAREA}
              aria-label={t("mobile.send.draft")}
              placeholder={t("mobile.send.placeholder")}
              rows={3}
              value={draft}
              onChange={(e) => {
                setDraft(e.target.value);
              }}
            />
            <span
              className={cx(
                "mono self-end text-[11px]",
                tooLong ? "text-danger" : "text-fg-subtle",
              )}
              data-testid="send-text-count">
              {t("mobile.send.count", { n: count, max: MAX_PHONE_TEXT_CHARS })}
            </span>
          </div>
          <div className="flex items-center gap-2">
            <Button
              icon="paste"
              className={TOUCH}
              onClick={() => {
                void sendClipboard();
              }}>
              {t("mobile.send.clipboard")}
            </Button>
            <Button
              variant="primary"
              className={cx(TOUCH, "min-w-0 flex-1")}
              disabled={draft.trim().length === 0 || tooLong}
              onClick={() => {
                void sendDraft();
              }}>
              <span className="truncate">
                {t("mobile.send.send", { name: target.device.name })}
              </span>
            </Button>
          </div>
        </>
      )}
      {sent.length > 0 && (
        <div className="flex flex-col border-t border-border pt-1">
          <div className="flex items-center justify-between">
            <span className="text-[12px] text-fg-muted">
              {t("mobile.send.sent", { n: sent.length })}
            </span>
            <Button
              variant="text-muted"
              className="h-11 text-[13px]"
              onClick={() => {
                void backend.invoke("sent_texts_clear");
              }}>
              {t("mobile.send.clear")}
            </Button>
          </div>
          <ul
            className="flex flex-col divide-y divide-border"
            aria-label={t("mobile.send.sentLabel")}>
            {sent.map((text) => (
              <li
                key={`${text.device}-${text.id}`}
                className="flex flex-col gap-1 py-2.5"
                data-testid="sent-text"
                data-state={text.state.state}>
                <span
                  className="line-clamp-2 text-[13px] leading-5 break-words text-fg"
                  data-user-text>
                  {text.body}
                </span>
                <span className="mono text-[11px] text-fg-subtle">
                  {formatDateTime(locale, text.sent_at, { timeStyle: "short" })} ·{" "}
                  {t(`mobile.send.source.${text.source}`)} · {text.device_name}
                </span>
                <StateLine tone={tone(text)} pulse={!sentTextFinal(text.state)} size="sm">
                  {sentTextLine(text, t)}
                </StateLine>
              </li>
            ))}
          </ul>
        </div>
      )}
    </Panel>
  );
}
