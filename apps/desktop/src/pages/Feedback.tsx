import {
  FEEDBACK_ATTACHMENT_ERRORS,
  FEEDBACK_ATTACHMENT_TYPES,
  FEEDBACK_CONTACT_MAX,
  FEEDBACK_ERRORS,
  FEEDBACK_KINDS,
  FEEDBACK_MAX_ATTACHMENTS,
  FEEDBACK_MAX_ATTACHMENT_TOTAL_BYTES,
  FEEDBACK_MAX_IMAGE_BYTES,
  FEEDBACK_MAX_VIDEO_BYTES,
  FEEDBACK_MESSAGE_MAX,
  type FeedbackAttachmentError,
  type FeedbackDiagnostics,
  type FeedbackError,
  type FeedbackInfo,
  type FeedbackKind,
  type Locale,
  PROVIDER_IDS,
  type ProviderId,
  type StagedAttachment,
  type TFunction,
  outputModeLabel,
} from "@voltip/shared";
import {
  Button,
  Icon,
  IconButton,
  Input,
  Keycap,
  Panel,
  Segmented,
  Textarea,
  useBackend,
  useI18n,
} from "@voltip/ui";
import { type ClipboardEvent, useCallback, useEffect, useRef, useState } from "react";
import { useFeedbackDraft } from "../app/feedback-draft";
import { openProjectLink } from "../app/project-links";
import { useRouter } from "../app/router";
import { useShell } from "../app/shell-context";
import { formatBytes } from "../features/update/download-rate";

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

/** The wire name of a refused attachment; anything else the shell said reads as a wrong type. */
export function attachmentError(error: unknown): FeedbackAttachmentError {
  const text = error instanceof Error ? error.message : String(error);
  return FEEDBACK_ATTACHMENT_ERRORS.find((e) => e === text) ?? "attachment_type";
}

/** The types a file with no type from the webview is taken for, by its extension. */
const EXTENSION_TYPES: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  mp4: "video/mp4",
  webm: "video/webm",
  mov: "video/quicktime",
};

/** A picked or pasted file's MIME type: the webview's, else the extension's, else none. */
export function attachmentType(file: { name: string; type: string }): string {
  if (file.type.length > 0) return file.type;
  const dot = file.name.lastIndexOf(".");
  return dot < 0 ? "" : (EXTENSION_TYPES[file.name.slice(dot + 1).toLowerCase()] ?? "");
}

/** The refusal the shell would give, told from the type and the size before the bytes are read
 *  (a large video is never loaded to be turned away); the shell checks again. */
export function precheckAttachment(
  type: string,
  size: number,
  staged: readonly StagedAttachment[],
): FeedbackAttachmentError | undefined {
  if (!(FEEDBACK_ATTACHMENT_TYPES as readonly string[]).includes(type)) return "attachment_type";
  const limit = type.startsWith("video/") ? FEEDBACK_MAX_VIDEO_BYTES : FEEDBACK_MAX_IMAGE_BYTES;
  if (size === 0 || size > limit) return "attachment_too_large";
  if (staged.length >= FEEDBACK_MAX_ATTACHMENTS) return "attachment_too_many";
  const total = staged.reduce((n, a) => n + a.size, 0) + size;
  return total > FEEDBACK_MAX_ATTACHMENT_TOTAL_BYTES ? "attachment_total" : undefined;
}

/** A limit in whole megabytes (`5 MB`), as the help line and the refusals word them. */
const megabytes = (bytes: number) => `${Math.round(bytes / 1_048_576)} MB`;
const LIMITS = {
  count: FEEDBACK_MAX_ATTACHMENTS,
  image: megabytes(FEEDBACK_MAX_IMAGE_BYTES),
  video: megabytes(FEEDBACK_MAX_VIDEO_BYTES),
  total: megabytes(FEEDBACK_MAX_ATTACHMENT_TOTAL_BYTES),
};

const TITLE_ID = "vt-feedback-title";
const MESSAGE_ID = "vt-feedback-message";

/** The 反馈 dialog (docs/feedback.md). The sidebar's 反馈 and 设置 · 关于 open the same dialog over
 *  the page beneath (user decision 2026-09-28; from 设置 it takes the settings dialog's place): a
 *  kind, the user's words, an optional contact and up to three screenshots or recordings on the
 *  left, the exact diagnostics that go along on the right, shown before anything is sent. The
 *  draft lives above the dialog (`FeedbackDraftProvider`), so Esc, the scrim or × close it without
 *  losing the words or the staged files; 清空 starts over. The shell stages the files and posts the
 *  report; the webview never learns where to. Once sent, a toast says so, the draft empties and
 *  the dialog closes. A build without an endpoint offers the repository's issue page instead. */
export function FeedbackDialog() {
  const { backend } = useBackend();
  const shell = useShell();
  const { navigate, background } = useRouter();
  const { t, locale } = useI18n();
  const { draft, update, discard, reset } = useFeedbackDraft();
  const { kind, message, contact, files } = draft;
  const [info, setInfo] = useState<FeedbackInfo | undefined>(undefined);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<FeedbackError | undefined>(undefined);
  const [adding, setAdding] = useState(false);
  const [refusal, setRefusal] = useState<
    { reason: FeedbackAttachmentError; name: string } | undefined
  >(undefined);
  const picker = useRef<HTMLInputElement>(null);
  const close = useCallback(() => {
    navigate(background);
  }, [navigate, background]);

  // Opening the dialog lands in the description, where the report starts.
  useEffect(() => {
    document.getElementById(MESSAGE_ID)?.focus();
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // The palette handles its own Esc first and marks it.
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.stopPropagation();
      close();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
    };
  }, [close]);

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
  const blank = message.length === 0 && contact.length === 0 && files.length === 0;
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
      // The report went out; only a file did not follow. Starting over would send it twice.
      if (reason === "attachments") reset();
      return;
    }
    setSending(false);
    reset();
    shell.toast({ message: t("feedback.sent"), duration: 4000 });
    close();
  };

  /** Stage `picked` in order, stopping at the first refusal, which is worded under the list. */
  const attach = async (picked: readonly File[]) => {
    if (picked.length === 0 || adding || sending) return;
    setAdding(true);
    setRefusal(undefined);
    let staged = files;
    // One at a time, in order: each file's limits depend on the ones staged before it.
    for (const file of picked) {
      const type = attachmentType(file);
      const early = precheckAttachment(type, file.size, staged);
      if (early !== undefined) {
        setRefusal({ reason: early, name: file.name });
        break;
      }
      try {
        // oxlint-disable-next-line no-await-in-loop -- sequential by design, see above
        const bytes = new Uint8Array(await file.arrayBuffer());
        // oxlint-disable-next-line no-await-in-loop -- sequential by design, see above
        const entry = await backend.feedbackAttachmentAdd({ name: file.name, type, bytes });
        staged = [...staged, entry];
        update({ files: (current) => [...current, entry] });
      } catch (e: unknown) {
        setRefusal({ reason: attachmentError(e), name: file.name });
        break;
      }
    }
    setAdding(false);
  };
  const detach = (id: string) => {
    update({ files: (current) => current.filter((f) => f.id !== id) });
    setRefusal(undefined);
    void backend.feedbackAttachmentRemove(id).catch(() => undefined);
  };
  const pasted = (e: ClipboardEvent) => {
    const list = [...e.clipboardData.files];
    if (list.length === 0) return;
    e.preventDefault();
    void attach(list);
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center scrim"
      onClick={close}
      data-testid="feedback-scrim">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={TITLE_ID}
        onClick={(e) => {
          e.stopPropagation();
        }}
        className="flex max-h-[calc(100vh-48px)] w-[min(960px,calc(100vw-48px))] flex-col overflow-hidden rounded-14 bg-surface hairline shadow-win"
        data-testid="feedback-dialog">
        <header className="flex h-12 shrink-0 items-center gap-4 border-b border-border px-6">
          <h2 id={TITLE_ID} className="text-[16px] font-semibold text-fg">
            {t("feedback.title")}
          </h2>
          <span className="mono ml-auto flex items-center gap-1.5 text-[10px] text-fg-subtle">
            <Keycap>Esc</Keycap> {t("settings.escClose")}
          </span>
          <IconButton icon="close" label={t("settings.close")} onClick={close} />
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto p-6">
          <p className="mb-4 text-[13px] text-fg-muted">{t("feedback.lede")}</p>
          <div className="grid grid-cols-1 items-start gap-4 md:grid-cols-[minmax(0,1fr)_minmax(240px,300px)]">
            <Panel
              eyebrow={t("feedback.formTitle")}
              bodyClassName="flex flex-col gap-4"
              onPaste={configured ? pasted : undefined}
              data-testid="feedback-form">
              <Segmented<FeedbackKind>
                label={t("feedback.kindLabel")}
                value={kind}
                onChange={(next) => {
                  update({ kind: next });
                }}
                options={FEEDBACK_KINDS.map((k) => ({ value: k, label: t(`feedback.kind.${k}`) }))}
                className="self-start"
              />
              <div className="flex flex-col gap-1">
                <Textarea
                  id={MESSAGE_ID}
                  label={t("feedback.messageLabel")}
                  placeholder={t("feedback.messagePlaceholder")}
                  value={message}
                  maxLength={FEEDBACK_MESSAGE_MAX}
                  rows={6}
                  onChange={(e) => {
                    update({ message: e.target.value });
                  }}
                />
                <span
                  className="mono self-end text-[11px] text-fg-subtle"
                  data-testid="feedback-count">
                  {t("feedback.count", { n: message.length, max: FEEDBACK_MESSAGE_MAX })}
                </span>
              </div>
              <Input
                label={t("feedback.contactLabel")}
                placeholder={t("feedback.contactPlaceholder")}
                value={contact}
                maxLength={FEEDBACK_CONTACT_MAX}
                onChange={(e) => {
                  update({ contact: e.target.value });
                }}
              />
              {configured && (
                <div className="flex flex-col gap-2" data-testid="feedback-attachments">
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-[12px] font-medium text-fg">
                      {t("feedback.attachLabel")}
                    </span>
                    <Button
                      size="sm"
                      icon="plus"
                      disabled={adding || sending || files.length >= FEEDBACK_MAX_ATTACHMENTS}
                      onClick={() => {
                        picker.current?.click();
                      }}
                      data-testid="feedback-attach">
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
                    <ul className="flex flex-col gap-1" aria-label={t("feedback.attachLabel")}>
                      {files.map((file) => {
                        const video = file.type.startsWith("video/");
                        return (
                          <li
                            key={file.id}
                            className="flex items-center gap-2 rounded-6 bg-inset px-2 py-1.5 text-[12px]"
                            data-testid="feedback-attachment">
                            <Icon
                              name={video ? "video" : "image"}
                              size={14}
                              className="shrink-0 text-fg-muted"
                              role="img"
                              aria-label={t(`feedback.attachKind.${video ? "video" : "image"}`)}
                            />
                            <span className="min-w-0 flex-1 truncate text-fg" title={file.name}>
                              {file.name}
                            </span>
                            <span className="mono shrink-0 text-fg-subtle">
                              {formatBytes(file.size)}
                            </span>
                            <IconButton
                              icon="x"
                              label={t("feedback.attachRemove", { name: file.name })}
                              disabled={sending || adding}
                              onClick={() => {
                                detach(file.id);
                              }}
                            />
                          </li>
                        );
                      })}
                    </ul>
                  )}
                  {refusal !== undefined && (
                    <p
                      role="alert"
                      className="text-[12px] text-danger"
                      data-testid="feedback-attach-error">
                      {t(`feedback.attachError.${refusal.reason}`, {
                        ...LIMITS,
                        name: refusal.name,
                      })}
                    </p>
                  )}
                  <p className="text-[11px] leading-4 text-fg-subtle">
                    {t("feedback.attachHelp", LIMITS)}
                  </p>
                </div>
              )}
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
              <div className="flex items-center justify-end gap-2">
                {configured ? (
                  <>
                    <Button
                      variant="ghost"
                      disabled={blank || sending || adding}
                      onClick={() => {
                        discard();
                        setError(undefined);
                        setRefusal(undefined);
                      }}
                      data-testid="feedback-discard">
                      {t("feedback.discard")}
                    </Button>
                    <Button
                      variant="primary"
                      disabled={empty || sending || adding || info === undefined}
                      onClick={() => {
                        void submit();
                      }}
                      data-testid="feedback-send">
                      {sending ? t("feedback.sending") : t("feedback.send")}
                    </Button>
                  </>
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
      </div>
    </div>
  );
}
