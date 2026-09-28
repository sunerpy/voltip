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
import { Button, Input, Panel, Segmented, Textarea, useBackend, useI18n } from "@voltip/ui";
import { useEffect, useState } from "react";
import { openProjectLink } from "../app/project-links";
import { useShell } from "../app/shell-context";

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

/** One diagnostics value as the page words it: provider and mode names, not wire ids. */
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

/** The 反馈 page (docs/feedback.md; a dialog until 2026-09-28, when every sidebar entry but 设置
 *  became a page of the main layout): a kind, the user's words and an optional contact on the left,
 *  the exact diagnostics that go along on the right, shown before anything is sent. The shell
 *  posts the report; the webview never learns where to. Once sent, a toast says so and the form
 *  clears. A build without an endpoint offers the repository's issue page instead. */
export function Feedback() {
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const [kind, setKind] = useState<FeedbackKind>("bug");
  const [message, setMessage] = useState("");
  const [contact, setContact] = useState("");
  const [info, setInfo] = useState<FeedbackInfo | undefined>(undefined);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<FeedbackError | undefined>(undefined);

  useEffect(() => {
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
  }, [backend, locale]);

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
    shell.toast({ message: t("feedback.sent"), duration: 4000 });
  };

  return (
    <div
      className="mx-auto flex w-full max-w-[1100px] flex-col gap-4 p-6"
      data-testid="page-feedback">
      <header>
        <h2 className="text-[18px] font-semibold text-fg">{t("feedback.title")}</h2>
        <p className="mt-1 text-[13px] text-fg-muted">{t("feedback.lede")}</p>
      </header>
      <div className="grid grid-cols-1 items-start gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(280px,340px)]">
        <Panel
          eyebrow={t("feedback.formTitle")}
          bodyClassName="flex flex-col gap-4"
          data-testid="feedback-form">
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
              rows={8}
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
          <div className="flex justify-end">
            {configured ? (
              <Button
                variant="primary"
                disabled={empty || sending || info === undefined}
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
                onClick={() => {
                  openProjectLink(backend, shell, "feedback");
                }}>
                {t("feedback.openIssue")}
              </Button>
            )}
          </div>
        </Panel>
        <Panel
          eyebrow={t("feedback.attached")}
          aria-label={t("feedback.attached")}
          role="region"
          bodyClassName="flex flex-col gap-2"
          data-testid="feedback-attached">
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
        </Panel>
      </div>
    </div>
  );
}
