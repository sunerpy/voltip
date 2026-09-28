import type {
  AppRef,
  ArgsOf,
  AudioDevice,
  FeedbackDraft,
  FeedbackInfo,
  FeedbackReceipt,
  InjectPreflight,
  LevelFrame,
  MutationCommand,
  PreviewDraft,
  Permission,
  PermissionReport,
  ProjectLink,
  ProviderId,
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
  /** What the OS grants right now (`permissions_status`, docs/dictation.md §15.1). */
  permissionsStatus(): Promise<PermissionReport>;
  /** Ask the OS for one permission (`permissions_request`); resolves once the request was issued
   *  (the answer arrives through the next `permissionsStatus`). */
  permissionsRequest(permission: Permission): Promise<void>;
  /** Would an injection into the current foreground window land (`inject_preflight`, §15.3). */
  injectPreflight(): Promise<InjectPreflight>;
  /** Open the vendor's API-key page in the browser (`provider_console_open`); the shell only opens
   *  catalogue URLs, so the webview names the provider, never a URL. */
  providerConsoleOpen(provider: ProviderId): Promise<void>;
  /** Open a project page in the browser (`project_link_open`): the repository or its new-issue
   *  page; the shell builds the URL from its own repository. */
  projectLinkOpen(link: ProjectLink): Promise<void>;
  /** What a 反馈 report would carry and whether this build can send it (`feedback_diagnostics`,
   *  docs/feedback.md); `locale` is the language the webview resolved. */
  feedbackDiagnostics(locale: string): Promise<FeedbackInfo>;
  /** Post a report (`feedback_submit`); rejects with a `FeedbackError` wire name. */
  feedbackSubmit(draft: FeedbackDraft): Promise<FeedbackReceipt>;
  /** The phone's clipboard text (`phone_clipboard_read`, docs/dictation.md §20.6), `null` when it
   *  holds none; rejects on a shell that has no phone clipboard. */
  phoneClipboardRead(): Promise<string | null>;
}
