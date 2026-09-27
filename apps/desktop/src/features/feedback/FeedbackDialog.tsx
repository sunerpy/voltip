import {
  FEEDBACK_CONTACT_MAX,
  FEEDBACK_ERRORS,
  FEEDBACK_KINDS,
  FEEDBACK_MESSAGE_MAX,
  type FeedbackDiagnostics,
  type FeedbackError,
  type FeedbackInfo,
  type FeedbackKind,
  type Locale,
  PROVIDER_IDS,
  type ProviderId,
  type TFunction,
  outputModeLabel,
} from "@voltip/shared";
import { Button, Dialog, Input, Segmented, Textarea, useBackend, useI18n } from "@voltip/ui";
import { useEffect, useState } from "react";

const OS_NAMES: Record<string, string> = { windows: "Windows", linux: "Linux", macos: "macOS" };
const DIAGNOSTIC_ORDER = [
  "app_version",
  "os",
  "arch",
  "session",
  "locale",
  "asr_provider",
  "local_model",
  "compute",
  "llm_provider",
  "output_mode",
] as const satisfies readonly (keyof FeedbackDiagnostics)[];

function isProvider(value: string): value is ProviderId {
  return (PROVIDER_IDS as readonly string[]).includes(value);
}

function isOutputMode(value: string): value is Parameters<typeof outputModeLabel>[0] {
  return value === "whole_take" || value === "streaming_final" || value === "live_inject";
}

/** One diagnostics value as the dialog words it: provider and mode names, not wire ids. */
export function diagnosticValue(
  key: (typeof DIAGNOSTIC_ORDER)[number],
  value: string,
  t: TFunction,
  locale: Locale,
): string {
  switch (key) {
    case "os":
      return OS_NAMES[value] ?? value;
    case "asr_provider":
    case "llm_provider":
      return isProvider(value) ? t(`engines.provider.${value}`) : value;
    case "compute":
      return value === "auto" || value === "cpu" || value === "gpu"
        ? t(`engines.compute.${value}`)
        : value;
    case "output_mode":
      return isOutputMode(value) ? outputModeLabel(value, locale) : value;
    default:
      return value;
  }
}

/** The wire name of a failed submission, or `server` for anything else the shell said. */
export function feedbackError(error: unknown): FeedbackError {
  const text = error instanceof Error ? error.message : String(error);
  return FEEDBACK_ERRORS.find((e) => e === text) ?? "server";
}

export interface FeedbackDialogProps {
  open: boolean;
  onClose: () => void;
  /** After the endpoint stored the report (the shell toasts). */
  onSent: () => void;
  /** A build without an endpoint: the repository's new-issue page instead. */
  onOpenIssue: () => void;
}

/** The 反馈 dialog (docs/feedback.md): a kind, the user's words, an optional contact, and the
 *  exact diagnostics that go along, shown before anything is sent. The shell posts the report;
 *  the webview never learns where to. */
export function FeedbackDialog({ open, onClose, onSent, onOpenIssue }: FeedbackDialogProps) {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const [kind, setKind] = useState<FeedbackKind>("bug");
  const [message, setMessage] = useState("");
  const [contact, setContact] = useState("");
  const [info, setInfo] = useState<FeedbackInfo | undefined>(undefined);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<FeedbackError | undefined>(undefined);

  useEffect(() => {
    if (!open) return;
    let live = true;
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
    };
  }, [open, backend, locale]);

  const configured = info?.configured ?? true;
  const empty = message.trim().length === 0;
  const submit = async () => {
    if (empty || sending || !configured) return;
    setSending(true);
    setError(undefined);
    const trimmed = contact.trim();
    try {
      await backend.feedbackSubmit({
        kind,
        message,
        contact: trimmed.length > 0 ? trimmed : null,
        locale,
      });
    } catch (e: unknown) {
      setSending(false);
      setError(feedbackError(e));
      return;
    }
    setSending(false);
    setMessage("");
    setContact("");
    onSent();
  };

  return (
    <Dialog
      open={open}
      title={t("feedback.title")}
      width={560}
      onClose={onClose}
      actions={
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          {configured ? (
            <Button
              size="sm"
              variant="primary"
              disabled={empty || sending || info === undefined}
              onClick={() => {
                void submit();
              }}
              data-testid="feedback-send">
              {sending ? t("feedback.sending") : t("feedback.send")}
            </Button>
          ) : (
            <Button size="sm" variant="primary" icon="external" onClick={onOpenIssue}>
              {t("feedback.openIssue")}
            </Button>
          )}
        </>
      }>
      <div className="flex flex-col gap-4" data-testid="feedback-dialog">
        <Segmented<FeedbackKind>
          label={t("feedback.kindLabel")}
          value={kind}
          onChange={setKind}
          options={FEEDBACK_KINDS.map((k) => ({ value: k, label: t(`feedback.kind.${k}`) }))}
          className="self-start"
        />
        <div className="flex flex-col gap-1">
          <Textarea
            label={t("feedback.messageLabel")}
            placeholder={t("feedback.messagePlaceholder")}
            value={message}
            maxLength={FEEDBACK_MESSAGE_MAX}
            rows={6}
            data-autofocus
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
          value={contact}
          maxLength={FEEDBACK_CONTACT_MAX}
          onChange={(e) => {
            setContact(e.target.value);
          }}
        />
        <section
          aria-label={t("feedback.attached")}
          className="flex flex-col gap-2 rounded-10 bg-inset p-3 hairline"
          data-testid="feedback-attached">
          <h3 className="text-[12px] font-medium text-fg">{t("feedback.attached")}</h3>
          {info === undefined ? (
            <p className="text-[12px] text-fg-muted">{t("feedback.loading")}</p>
          ) : (
            <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-[12px]">
              {DIAGNOSTIC_ORDER.flatMap((key) => {
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
          <p className="text-[11px] leading-4 text-fg-subtle">{t("feedback.attachedHelp")}</p>
        </section>
        {!configured && (
          <p className="text-[12px] text-fg-muted" data-testid="feedback-not-configured">
            {t("feedback.notConfigured")}
          </p>
        )}
        {error !== undefined && (
          <p role="alert" className="text-[12px] text-danger" data-testid="feedback-error">
            {t(`feedback.error.${error}`, { max: FEEDBACK_MESSAGE_MAX })}
          </p>
        )}
      </div>
    </Dialog>
  );
}
