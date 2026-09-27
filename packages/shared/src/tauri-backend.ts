import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { z } from "zod";
import { listen as tauriListen } from "@tauri-apps/api/event";

import type { Backend, EventListener, FrameListener, Unsubscribe } from "./backend";
import {
  type ArgsOf,
  type FeedbackDraft,
  type MutationCommand,
  type PreviewDraft,
  type Permission,
  type ProjectLink,
  type ProviderId,
  UI_EVENT_NAME,
  appRefSchema,
  audioDeviceSchema,
  feedbackInfoSchema,
  feedbackReceiptSchema,
  injectPreflightSchema,
  levelFrameSchema,
  permissionReportSchema,
  uiEventSchema,
  uiStateSchema,
  updateStatusSchema,
  vocabularyPreviewSchema,
} from "./schema";

/** The part of `@tauri-apps/api/core` `Channel` the backend needs; injectable because the real one
 *  registers itself with the Tauri runtime on construction and cannot exist in a plain browser. */
export interface ChannelLike {
  onmessage: (raw: unknown) => void;
}

type InvokeFn = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
type ListenFn = (
  event: string,
  handler: (event: { payload: unknown }) => void,
) => Promise<() => void>;

export interface TauriTransport {
  invoke: InvokeFn;
  listen: ListenFn;
  /** Creates the streaming channel handed to `audio_meter_start`; defaults to Tauri's `Channel`. */
  channel?: () => ChannelLike;
  /** Where dropped events are reported; defaults to `console.warn`. */
  warn?: (message: string, detail: unknown) => void;
}

const newChannel = (): ChannelLike => new Channel();

const defaultTransport: TauriTransport = {
  invoke: (command, args) => tauriInvoke(command, args),
  listen: (event, handler) => tauriListen<unknown>(event, handler),
  channel: newChannel,
};

/** Real backend over `@tauri-apps/api`. Payloads are validated with zod before they reach the UI. */
export class TauriBackend implements Backend {
  private readonly transport: TauriTransport;

  constructor(transport: Partial<TauriTransport> = {}) {
    this.transport = { ...defaultTransport, ...transport };
  }

  async getState() {
    const raw = await this.transport.invoke("core_state");
    return uiStateSchema.parse(raw);
  }

  async invoke<C extends MutationCommand>(name: C, ...args: ArgsOf<C>): Promise<void> {
    const [payload] = args;
    await this.transport.invoke(name, payload);
  }

  async audioDevices() {
    const raw = await this.transport.invoke("audio_devices");
    return audioDeviceSchema.array().parse(raw);
  }

  async updateStatus() {
    const raw = await this.transport.invoke("update_status");
    return updateStatusSchema.parse(raw);
  }

  async vocabularyPreview(text: string, draft?: PreviewDraft) {
    const raw = await this.transport.invoke("vocabulary_preview", { text, draft: draft ?? null });
    return vocabularyPreviewSchema.parse(raw);
  }

  async rulesExport() {
    const raw = await this.transport.invoke("rules_export");
    return z.string().parse(raw);
  }

  async recentApps() {
    const raw = await this.transport.invoke("recent_apps");
    return appRefSchema.array().parse(raw);
  }

  async permissionsStatus() {
    const raw = await this.transport.invoke("permissions_status");
    return permissionReportSchema.parse(raw);
  }

  async permissionsRequest(permission: Permission): Promise<void> {
    await this.transport.invoke("permissions_request", { permission });
  }

  async providerConsoleOpen(provider: ProviderId): Promise<void> {
    await this.transport.invoke("provider_console_open", { provider });
  }

  async projectLinkOpen(link: ProjectLink): Promise<void> {
    await this.transport.invoke("project_link_open", { link });
  }

  async feedbackDiagnostics(locale: string) {
    const raw = await this.transport.invoke("feedback_diagnostics", { locale });
    return feedbackInfoSchema.parse(raw);
  }

  async feedbackSubmit(draft: FeedbackDraft) {
    const raw = await this.transport.invoke("feedback_submit", { ...draft });
    return feedbackReceiptSchema.parse(raw);
  }

  async injectPreflight() {
    const raw = await this.transport.invoke("inject_preflight");
    return injectPreflightSchema.parse(raw);
  }

  async meter(deviceId: string | undefined, onFrame: FrameListener): Promise<Unsubscribe> {
    const warn = this.warn();
    const channel = (this.transport.channel ?? newChannel)();
    let stopped = false;
    // Tauri's `Channel` is not an EventTarget: `onmessage` is its only delivery hook.
    // oxlint-disable-next-line unicorn/prefer-add-event-listener
    channel.onmessage = (raw) => {
      if (stopped) return;
      const parsed = levelFrameSchema.safeParse(raw);
      if (parsed.success) onFrame(parsed.data);
      else warn("audio meter frame dropped: payload failed validation", parsed.error.issues);
    };
    // The shell owns the microphone (one hub, many subscribers): the id names this subscription.
    const raw = await this.transport.invoke("audio_meter_start", {
      deviceId: deviceId ?? null,
      onFrame: channel,
    });
    const id = z.number().int().nonnegative().parse(raw);
    return () => {
      if (stopped) return;
      stopped = true;
      void this.transport.invoke("audio_meter_stop", { id }).catch((e: unknown) => {
        warn("audio_meter_stop failed", e);
      });
    };
  }

  private warn(): (message: string, detail: unknown) => void {
    return (
      this.transport.warn ??
      ((m, d) => {
        console.warn(m, d);
      })
    );
  }

  on(listener: EventListener): Unsubscribe {
    let disposed = false;
    const warn = this.warn();
    const ready = this.transport.listen(UI_EVENT_NAME, ({ payload }) => {
      if (disposed) return;
      const parsed = uiEventSchema.safeParse(payload);
      if (parsed.success) listener(parsed.data);
      else warn("voltip://event dropped: payload failed validation", parsed.error.issues);
    });
    return () => {
      disposed = true;
      void ready.then((unlisten) => {
        unlisten();
      });
    };
  }
}
