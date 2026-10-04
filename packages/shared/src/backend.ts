import type {
  AppRef,
  ArgsOf,
  AttachmentFile,
  AudioDevice,
  AudioOutputs,
  BuiltinPresetText,
  ExportFormat,
  ExportOutcome,
  BuiltinSceneTerms,
  FeedbackDraft,
  FeedbackInfo,
  FeedbackReceipt,
  HistoryEntry,
  HistoryHits,
  HistoryPage,
  HistoryQueryArgs,
  HistoryStats,
  InjectPreflight,
  MirrorEntry,
  MirrorProfile,
  LevelFrame,
  MutationCommand,
  PasteOutcome,
  PreviewDraft,
  Permission,
  PermissionReport,
  GuidePage,
  ProjectLink,
  ProviderId,
  StagedAttachment,
  UiEvent,
  UiState,
  UpdateStatus,
  VocabularyPreview,
} from "./schema";

export type EventListener = (event: UiEvent) => void;
export type Unsubscribe = () => void;
export type FrameListener = (frame: LevelFrame) => void;

/** What the webviews talk to. Implemented by `TauriBackend` (real) and `MockBackend` (in-memory). */
export interface Backend {
  /** Everything the UI renders right now (`core_state`). */
  getState(): Promise<UiState>;
  /** Dispatch a command; resolves once the core accepted it. */
  invoke<C extends MutationCommand>(name: C, ...args: ArgsOf<C>): Promise<void>;
  /** Subscribe to validated `voltip://event` payloads. */
  on(listener: EventListener): Unsubscribe;
  /** Microphones the native audio backend can open (`audio_devices`); default device first. */
  audioDevices(): Promise<AudioDevice[]>;
  /** Whether the computer's sound can be recorded here and the output devices it can be recorded
   *  from (`audio_outputs`, docs/dictation.md §22). */
  audioOutputs(): Promise<AudioOutputs>;
  /** 导出字幕（SRT）/ 导出文本（TXT） of entry `id` (`history_export`, docs/dictation.md §22): the
   *  shell's save dialog offers `fileName`, and the shell writes the file. */
  historyExport(id: string, format: ExportFormat, fileName: string): Promise<ExportOutcome>;
  /** Start the native input level meter on `deviceId` (default device when `undefined`). Frames
   *  stream at ≈ 30 Hz through a Tauri `Channel`; the returned function stops the meter. */
  meter(deviceId: string | undefined, onFrame: FrameListener): Promise<Unsubscribe>;
  /** The updater's current status (`update_status`); the same value `UiState.update` carries. */
  updateStatus(): Promise<UpdateStatus>;
  /** `text` through the personal dictionary and the replacement rules exactly as a take would run
   *  them, minus the LLM (`vocabulary_preview`, docs/dictation.md §16.4); `draft` stands in for the
   *  rule it names (or is appended). Rejects with the core's message when the draft is invalid (a
   *  regex that does not compile) or the text is over `MAX_TEXT_BYTES`. */
  vocabularyPreview(text: string, draft?: PreviewDraft): Promise<VocabularyPreview>;
  /** The replacement rules as TOML text (`rules_export`, §16.5). */
  rulesExport(): Promise<string>;
  /** The applications the history saw, newest first, one per id (`recent_apps`,
   *  docs/dictation.md §18.6): what the scene editor offers to pick from. */
  recentApps(): Promise<AppRef[]>;
  /** A page of the history, filtered, searched and paged by the core (`history_query`,
   *  docs/dictation.md §4.4). Rejects when `limit` is outside 1–`HISTORY_QUERY_LIMIT`. */
  historyQuery(args: HistoryQueryArgs): Promise<HistoryPage>;
  /** One history entry, `null` once it is gone (`history_entry`). */
  historyEntry(id: string): Promise<HistoryEntry | null>;
  /** The dictations between each two of `boundaries` (local midnights, increasing, 2–43) and
   *  over the whole history (`history_stats`, §4.5). */
  historyStats(boundaries: readonly number[]): Promise<HistoryStats>;
  /** How often each dictionary entry and rule fired in the history (`history_hits`, §16.3). */
  historyHits(): Promise<HistoryHits>;
  /** Phone (docs/dictation.md §20.8): a page of the copy of computer `desktop`'s history (its key
   *  in hex), read like the phone's own; empty when there is no copy (`mirror_history_query`). */
  mirrorHistoryQuery(desktop: string, args: HistoryQueryArgs): Promise<HistoryPage>;
  /** Phone: one entry of the copy and whether it arrived shortened, `null` once it is gone. */
  mirrorHistoryEntry(desktop: string, id: string): Promise<MirrorEntry | null>;
  /** Phone: the computer's settings as the copy holds them (设置 › 电脑), `null` before any arrived. */
  mirrorProfile(desktop: string): Promise<MirrorProfile | null>;
  /** What the OS grants right now (`permissions_status`, docs/dictation.md §15.1). */
  permissionsStatus(): Promise<PermissionReport>;
  /** Ask the OS for one permission (`permissions_request`); resolves once the request was issued
   *  (the answer arrives through the next `permissionsStatus`). */
  permissionsRequest(permission: Permission): Promise<void>;
  /** Would an injection into the current foreground window land (`inject_preflight`, §15.3). */
  injectPreflight(): Promise<InjectPreflight>;
  /** 「粘贴到上一个窗口」 (`paste_text`): paste `text` into the window the user came from, or copy
   *  it; resolves with what became of it (a refusal is an outcome, not a rejection). */
  pasteText(text: string): Promise<PasteOutcome>;
  /** Open the vendor's API-key page in the browser (`provider_console_open`); the shell only opens
   *  catalogue URLs, so the webview names the provider, never a URL. */
  providerConsoleOpen(provider: ProviderId): Promise<void>;
  /** Open a project page in the browser (`project_link_open`): the repository or its new-issue
   *  page; the shell builds the URL from its own repository. */
  projectLinkOpen(link: ProjectLink): Promise<void>;
  /** Open a page of the user guide in the browser (`guide_open`), in `locale`'s language; the
   *  shell builds the URL. */
  guideOpen(page: GuidePage, locale: string): Promise<void>;
  /** `model_folder_open` (docs/dictation.md §10): the model's directory in the file manager,
   *  created when missing. */
  modelFolderOpen(id: string): Promise<void>;
  /** `model_link_open`: `files[file].urls[source]` of the model in the browser; the shell takes
   *  the address from the core's catalogue. */
  modelLinkOpen(id: string, file: string, source: number): Promise<void>;
  /** What a 反馈 report would carry and whether this build can send it (`feedback_diagnostics`,
   *  docs/feedback.md); `locale` is the language the webview resolved. */
  feedbackDiagnostics(locale: string): Promise<FeedbackInfo>;
  /** Post a report (`feedback_submit`); rejects with a `FeedbackError` wire name. */
  feedbackSubmit(draft: FeedbackDraft): Promise<FeedbackReceipt>;
  /** Stage a screenshot or a screen recording for the next report (`feedback_attachment_add`);
   *  rejects with a `FeedbackAttachmentError` wire name. */
  feedbackAttachmentAdd(file: AttachmentFile): Promise<StagedAttachment>;
  /** Drop a staged file (`feedback_attachment_remove`). */
  feedbackAttachmentRemove(id: string): Promise<void>;
  /** Drop every staged file (`feedback_attachments_clear`). */
  feedbackAttachmentsClear(): Promise<void>;
  /** The phone's clipboard text (`phone_clipboard_read`, docs/dictation.md §20.6), `null` when it
   *  holds none; rejects on a shell that has no phone clipboard. */
  phoneClipboardRead(): Promise<string | null>;
  /** Every built-in preset's text (`presets_builtin`, docs/dictation.md §21): what 复制为自定义
   *  starts from; rejects on the phone, which has no presets. */
  presetsBuiltin(): Promise<BuiltinPresetText[]>;
  /** Every built-in scene's term pack (`scenes_builtin`, docs/dictation.md §18.10): what 查看术语
   *  lists; rejects on the phone, which has no scenes. */
  scenesBuiltin(): Promise<BuiltinSceneTerms[]>;
}
