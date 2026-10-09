import {
  FEEDBACK_ATTACHMENT_TYPES,
  FEEDBACK_CONTACT_MAX,
  FEEDBACK_DIAGNOSTIC_ORDER,
  FEEDBACK_KINDS,
  FEEDBACK_LIMITS,
  FEEDBACK_MAX_ATTACHMENTS,
  FEEDBACK_MESSAGE_MAX,
  type FeedbackAttachmentError,
  type FeedbackError,
  type FeedbackInfo,
  type FeedbackKind,
  type StagedAttachment,
  attachmentError,
  attachmentSize,
  attachmentType,
  diagnosticValue,
  feedbackError,
  precheckAttachment,
} from "@voltip/shared";
import {
  Button,
  Card,
  Icon,
  IconButton,
  Input,
  Panel,
  Segmented,
  Textarea,
  useBackend,
  useI18n,
} from "@voltip/ui";
import { useEffect, useRef, useState } from "react";
import { Lede, PAGE, TOUCH, TOUCH_ICON, TOUCH_TEXTAREA } from "../app/phone-ui";
import { useMobileShell } from "../app/shell";

/** 反馈 on the phone (docs/feedback.md; user decision 2026-10-01: the phone sends feedback of its
 *  own): the desktop's form in one column — a kind, the user's words, an optional contact and up
 *  to three screenshots or recordings from the photo picker — and below it the exact diagnostics
 *  that go along, shown before anything is sent. The shell stages the files and posts the report;
 *  the page never learns where to. The page opens with nothing staged and takes its files along
 *  when it is left. A build without an endpoint offers the repository's issue page instead. */
export function Feedback() {
  const { backend } = useBackend();
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  const [kind, setKind] = useState<FeedbackKind>("bug");
  const [message, setMessage] = useState("");
  const [contact, setContact] = useState("");
  const [files, setFiles] = useState<StagedAttachment[]>([]);
  const [info, setInfo] = useState<FeedbackInfo | undefined>(undefined);
  const [sending, setSending] = useState(false);
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<FeedbackError | undefined>(undefined);
  const [refusal, setRefusal] = useState<
    { reason: FeedbackAttachmentError; name: string } | undefined
  >(undefined);
  const picker = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let live = true;
    void backend.feedbackAttachmentsClear().catch(() => undefined);
    void backend
      .feedbackDiagnostics(locale)
      .then((answer) => {
        if (live) setInfo(answer);
      })
      .catch(() => {
        if (live) setError("server");
      });
    return () => {
      live = false;
      void backend.feedbackAttachmentsClear().catch(() => undefined);
    };
  }, [backend, locale]);

  const configured = info?.configured ?? true;
  const empty = message.trim().length === 0;
  const submit = async () => {
    if (empty || sending || adding || !configured) return;
    setSending(true);
    setError(undefined);
    setRefusal(undefined);
    const trimmed = contact.trim();
    try {
      await backend.feedbackSubmit({
        kind,
        message,
        contact: trimmed.length > 0 ? trimmed : null,
        locale,
        ...(files.length > 0 ? { attachments: files.map((f) => f.id) } : {}),
      });
    } catch (e: unknown) {
      const reason = feedbackError(e);
      setSending(false);
      setError(reason);
      // The report went out; only a file did not follow. Sending again would send it twice.
      if (reason === "attachments") {
        setMessage("");
        setContact("");
        setFiles([]);
      }
      return;
    }
    setSending(false);
    shell.toast(t("feedback.sent"));
    shell.back();
  };

  /** Stage `picked` in order, stopping at the first refusal, which is worded under the list. */
  const attach = async (picked: readonly File[]) => {
    if (picked.length === 0 || adding || sending) return;
    setAdding(true);
    setRefusal(undefined);
    let staged = files;
    for (const file of picked) {
      const type = attachmentType(file);
      const early = precheckAttachment(type, file.size, staged);
      if (early !== undefined) {
        setRefusal({ reason: early, name: file.name });
        break;
      }
      try {
        // oxlint-disable-next-line no-await-in-loop -- one at a time: each file's limits depend on the ones before
        const bytes = new Uint8Array(await file.arrayBuffer());
        // oxlint-disable-next-line no-await-in-loop -- as above
        const entry = await backend.feedbackAttachmentAdd({ name: file.name, type, bytes });
        staged = [...staged, entry];
        setFiles(staged);
      } catch (e: unknown) {
        setRefusal({ reason: attachmentError(e), name: file.name });
        break;
      }
    }
    setAdding(false);
  };
  const detach = (id: string) => {
    setFiles((current) => current.filter((f) => f.id !== id));
    setRefusal(undefined);
    void backend.feedbackAttachmentRemove(id).catch(() => undefined);
  };

  return (
    <div className={PAGE} data-testid="phone-feedback">
      <Lede>{t("mobile.feedback.lede")}</Lede>
      <Card className="flex flex-col gap-4">
        <Segmented<FeedbackKind>
          label={t("feedback.kindLabel")}
          value={kind}
          onChange={setKind}
          options={FEEDBACK_KINDS.map((k) => ({ value: k, label: t(`feedback.kind.${k}`) }))}
          className="h-11 w-full [&>button]:flex-1"
        />
        <div className="flex flex-col gap-1">
          <Textarea
            label={t("feedback.messageLabel")}
            className={TOUCH_TEXTAREA}
            placeholder={t("feedback.messagePlaceholder")}
            value={message}
            maxLength={FEEDBACK_MESSAGE_MAX}
            rows={6}
            onChange={(e) => {
              setMessage(e.target.value);
            }}
          />
          <span className="mono self-end text-[11px] text-fg-subtle" data-testid="feedback-count">
            {t("feedback.count", { n: message.length, max: FEEDBACK_MESSAGE_MAX })}
          </span>
        </div>
        <Input
          label={t("feedback.contactLabel")}
          placeholder={t("feedback.contactPlaceholder")}
          size="lg"
          value={contact}
          maxLength={FEEDBACK_CONTACT_MAX}
          onChange={(e) => {
            setContact(e.target.value);
          }}
        />
      </Card>
      {configured && (
        <div className="flex flex-col gap-2" data-testid="feedback-attachments">
          <div className="flex items-center justify-between gap-3 pl-1">
            <span className="eyebrow">{t("feedback.attachLabel")}</span>
            <Button
              icon="plus"
              className={TOUCH}
              disabled={adding || sending || files.length >= FEEDBACK_MAX_ATTACHMENTS}
              onClick={() => {
                picker.current?.click();
              }}>
              {adding ? t("feedback.attachAdding") : t("feedback.attachAdd")}
            </Button>
            <input
              ref={picker}
              type="file"
              multiple
              hidden
              accept={FEEDBACK_ATTACHMENT_TYPES.join(",")}
              data-testid="feedback-attach-input"
              onChange={(e) => {
                const list = [...(e.target.files ?? [])];
                // The same file picked again is a new change.
                e.target.value = "";
                void attach(list);
              }}
            />
          </div>
          {files.length > 0 && (
            <Card padding="none" className="overflow-hidden">
              <ul
                className="flex flex-col divide-y divide-border"
                aria-label={t("feedback.attachLabel")}>
                {files.map((file) => {
                  const video = file.type.startsWith("video/");
                  return (
                    <li
                      key={file.id}
                      className="flex min-h-12 items-center gap-2 pr-1 pl-4 text-[13px]"
                      data-testid="feedback-attachment">
                      <Icon
                        name={video ? "video" : "image"}
                        size={16}
                        className="shrink-0 text-fg-muted"
                        role="img"
                        aria-label={t(`feedback.attachKind.${video ? "video" : "image"}`)}
                      />
                      <span className="min-w-0 flex-1 truncate text-fg">{file.name}</span>
                      <span className="mono shrink-0 text-[11px] text-fg-subtle">
                        {attachmentSize(file.size)}
                      </span>
                      <IconButton
                        icon="x"
                        size={28}
                        label={t("feedback.attachRemove", { name: file.name })}
                        className={TOUCH_ICON}
                        disabled={sending || adding}
                        onClick={() => {
                          detach(file.id);
                        }}
                      />
                    </li>
                  );
                })}
              </ul>
            </Card>
          )}
          {refusal !== undefined && (
            <p
              role="alert"
              className="px-1 text-[12px] leading-5 text-danger"
              data-testid="feedback-attach-error">
              {t(`feedback.attachError.${refusal.reason}`, {
                ...FEEDBACK_LIMITS,
                name: refusal.name,
              })}
            </p>
          )}
          <p className="px-1 text-[12px] leading-5 text-fg-subtle">
            {t("mobile.feedback.attachHelp", FEEDBACK_LIMITS)}
          </p>
        </div>
      )}
      {!configured && (
        <p
          className="px-1 text-[12px] leading-5 text-fg-muted"
          data-testid="feedback-not-configured">
          {t("feedback.notConfigured")}
        </p>
      )}
      {error !== undefined && (
        <p
          role="alert"
          className="px-1 text-[12px] leading-5 text-danger"
          data-testid="feedback-error">
          {t(`feedback.error.${error}`, { max: FEEDBACK_MESSAGE_MAX })}
        </p>
      )}
      {configured ? (
        <Button
          variant="primary"
          className={`${TOUCH} w-full`}
          disabled={empty || sending || adding || info === undefined}
          onClick={() => {
            void submit();
          }}
          data-testid="feedback-send">
          {sending ? t("feedback.sending") : t("feedback.send")}
        </Button>
      ) : (
        <Button
          variant="primary"
          icon="external"
          className={`${TOUCH} w-full`}
          onClick={() => {
            backend.projectLinkOpen("feedback").catch((e: unknown) => {
              shell.toast(
                t("mobile.toast.error", { message: e instanceof Error ? e.message : String(e) }),
                "danger",
              );
            });
          }}>
          {t("feedback.openIssue")}
        </Button>
      )}
      <section aria-label={t("feedback.attached")} data-testid="feedback-attached">
        <Panel eyebrow={t("feedback.attached")} bodyClassName="flex flex-col gap-3">
          {info === undefined ? (
            <p className="text-[12px] text-fg-muted">{t("feedback.loading")}</p>
          ) : (
            <dl className="grid grid-cols-[max-content_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-[12px]">
              {FEEDBACK_DIAGNOSTIC_ORDER.flatMap((key) => {
                const value = info.diagnostics[key];
                if (value === undefined) return [];
                return [
                  <dt key={`${key}-k`} className="text-fg-muted">
                    {t(`feedback.diag.${key}`)}
                  </dt>,
                  <dd key={`${key}-v`} className="mono truncate text-fg" data-diagnostic={key}>
                    {diagnosticValue(key, value, t, locale)}
                  </dd>,
                ];
              })}
            </dl>
          )}
          <p className="text-[12px] leading-5 text-fg-subtle">{t("feedback.attachedHelp")}</p>
        </Panel>
      </section>
    </div>
  );
}
