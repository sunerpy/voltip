import {
  type ConnectionState,
  type TauriTransport,
  type UiEvent,
  type UiState,
  TauriBackend,
  applyEvent,
  notApplicablePermissions,
  uncheckedPreflight,
} from "@voltip/shared";
import { MockBackend, sampleDevices } from "@voltip/shared/mock";
import { act, render, renderHook, screen, waitFor } from "@testing-library/react";
import { BackendProvider, useBackend, useUiState } from "./BackendProvider";

function Probe() {
  const { state, error, lastEvent } = useBackend();
  const ui = useUiState();
  return (
    <div>
      <span data-testid="name">{state?.identity?.name ?? "loading"}</span>
      <span data-testid="devices">{ui.devices.length}</span>
      <span data-testid="error">{error ?? ""}</span>
      <span data-testid="last">{lastEvent?.type ?? ""}</span>
    </div>
  );
}

describe("BackendProvider", () => {
  it("loads core_state, folds events and exposes the last event", async () => {
    const backend = new MockBackend({ devices: sampleDevices(1) });
    const onEvent = vi.fn();
    render(
      <BackendProvider backend={backend} onEvent={onEvent}>
        <Probe />
      </BackendProvider>,
    );
    expect(screen.getByTestId("name")).toHaveTextContent("loading");
    expect(screen.getByTestId("devices")).toHaveTextContent("0");
    await waitFor(() => {
      expect(screen.getByTestId("name")).toHaveTextContent("Surface-Laptop");
    });
    expect(screen.getByTestId("devices")).toHaveTextContent("2");
    await act(async () => {
      await backend.invoke("device_rename", { name: "Studio" });
    });
    expect(screen.getByTestId("name")).toHaveTextContent("Studio");
    expect(screen.getByTestId("last")).toHaveTextContent("identity");
    expect(onEvent).toHaveBeenCalledTimes(1);
  });

  it("waits for core_state while only partial events have come, then shows its answer", async () => {
    let resolve: ((s: Awaited<ReturnType<MockBackend["getState"]>>) => void) | undefined;
    const inner = new MockBackend();
    const backend = {
      getState: () =>
        new Promise<Awaited<ReturnType<MockBackend["getState"]>>>((r) => {
          resolve = r;
        }),
      invoke: inner.invoke.bind(inner),
      on: inner.on.bind(inner),
      audioDevices: inner.audioDevices.bind(inner),
      audioOutputs: inner.audioOutputs.bind(inner),
      historyExport: inner.historyExport.bind(inner),
      importsState: inner.importsState.bind(inner),
      importPick: inner.importPick.bind(inner),
      onFileDrop: inner.onFileDrop.bind(inner),
      backupExport: inner.backupExport.bind(inner),
      backupOpen: inner.backupOpen.bind(inner),
      backupImport: inner.backupImport.bind(inner),
      backupDiscard: inner.backupDiscard.bind(inner),
      backupFolderChoose: inner.backupFolderChoose.bind(inner),
      backupFolderOpen: inner.backupFolderOpen.bind(inner),
      meter: inner.meter.bind(inner),
      updateStatus: inner.updateStatus.bind(inner),
      vocabularyPreview: inner.vocabularyPreview.bind(inner),
      vocabularyBacktest: inner.vocabularyBacktest.bind(inner),
      vocabularySuggestions: inner.vocabularySuggestions.bind(inner),
      vocabularySuggestionAccept: inner.vocabularySuggestionAccept.bind(inner),
      vocabularySuggestionUndo: inner.vocabularySuggestionUndo.bind(inner),
      vocabularySuggestionsAccept: inner.vocabularySuggestionsAccept.bind(inner),
      vocabularySuggestionsIgnore: inner.vocabularySuggestionsIgnore.bind(inner),
      vocabularySuggestionsUndo: inner.vocabularySuggestionsUndo.bind(inner),
      rulesWarnings: inner.rulesWarnings.bind(inner),
      rulesExport: inner.rulesExport.bind(inner),
      recentApps: inner.recentApps.bind(inner),
      historyQuery: inner.historyQuery.bind(inner),
      historyEntry: inner.historyEntry.bind(inner),
      historyStats: inner.historyStats.bind(inner),
      historyHits: inner.historyHits.bind(inner),
      mirrorHistoryQuery: inner.mirrorHistoryQuery.bind(inner),
      mirrorHistoryEntry: inner.mirrorHistoryEntry.bind(inner),
      mirrorProfile: inner.mirrorProfile.bind(inner),
      permissionsStatus: inner.permissionsStatus.bind(inner),
      permissionsRequest: inner.permissionsRequest.bind(inner),
      injectPreflight: inner.injectPreflight.bind(inner),
      pasteText: inner.pasteText.bind(inner),
      overlayCopy: inner.overlayCopy.bind(inner),
      overlayPaste: inner.overlayPaste.bind(inner),
      openHistory: inner.openHistory.bind(inner),
      providerConsoleOpen: inner.providerConsoleOpen.bind(inner),
      projectLinkOpen: inner.projectLinkOpen.bind(inner),
      guideOpen: inner.guideOpen.bind(inner),
      modelFolderOpen: inner.modelFolderOpen.bind(inner),
      modelLinkOpen: inner.modelLinkOpen.bind(inner),
      modelsLocationChoose: inner.modelsLocationChoose.bind(inner),
      feedbackDiagnostics: () => Promise.reject(new Error("no feedback")),
      feedbackSubmit: () => Promise.reject(new Error("no feedback")),
      feedbackAttachmentAdd: () => Promise.reject(new Error("no feedback")),
      feedbackAttachmentRemove: () => Promise.resolve(),
      feedbackAttachmentsClear: () => Promise.resolve(),
      phoneClipboardRead: () => Promise.resolve(null),
      presetsBuiltin: () => Promise.resolve([]),
      scenesBuiltin: () => Promise.resolve([]),
    };
    render(
      <BackendProvider backend={backend}>
        <Probe />
      </BackendProvider>,
    );
    act(() => {
      inner.simulateRelay({ state: "connected" });
    });
    expect(screen.getByTestId("name")).toHaveTextContent("loading");
    act(() => {
      inner.simulateError("x");
    });
    const full = inner.peek();
    act(() => {
      for (const l of [backend]) void l;
      inner.on(() => undefined);
    });
    await act(async () => {
      resolve?.({ ...full, secret_backend: "late" });
      await Promise.resolve();
    });
    expect(screen.getByTestId("name")).toHaveTextContent("Surface-Laptop");
  });

  describe("regression: the relay a window shows once the relay is up (user report 2026-11-02)", () => {
    // A window opened while the relay connected showed 认证中 until Voltip was restarted, though
    // the connection was up: the change fell between the window's first state and its events.

    /** The shell as a window meets it through Tauri: `core_state` answers with the core's state
     *  when it is asked, an event reaches only a listener whose registration the shell has
     *  finished, and both `listen` and `core_state` answer after a round trip, here when the test
     *  says so. */
    function shell(relay: ConnectionState) {
      let state: UiState = { ...new MockBackend().peek(), relay: relayAt(relay) };
      const live = new Set<(e: { payload: unknown }) => void>();
      const registrations: (() => void)[] = [];
      const answers: (() => void)[] = [];
      let holding = false;
      const transport: Partial<TauriTransport> = {
        invoke: (command) => {
          if (command !== "core_state") return Promise.resolve(null);
          const snapshot = structuredClone(state);
          if (!holding) return Promise.resolve(snapshot);
          return new Promise((resolve) => {
            answers.push(() => {
              resolve(snapshot);
            });
          });
        },
        listen: (_event, handler) =>
          new Promise((resolve) => {
            registrations.push(() => {
              live.add(handler);
              resolve(() => {
                live.delete(handler);
              });
            });
          }),
      };
      return {
        backend: new TauriBackend(transport),
        /** The core folds `event` into its state, and the shell sends it to the live listeners. */
        emit(event: UiEvent) {
          state = applyEvent(state, event);
          for (const handler of [...live]) handler({ payload: event });
        },
        /** The registrations on their way reach the shell. */
        register() {
          for (const done of registrations.splice(0)) done();
        },
        /** From now on `core_state` waits for `answer`. */
        hold() {
          holding = true;
        },
        answer() {
          for (const done of answers.splice(0)) done();
        },
      };
    }

    function relayAt(state: ConnectionState) {
      return { state, attempts: 0, source: "user" as const, endpoint: "ws://127.0.0.1:47830" };
    }

    function relayEvent(state: ConnectionState): UiEvent {
      return { type: "relay", ...relayAt(state) };
    }

    function Relay() {
      return <span data-testid="relay">{useBackend().state?.relay.state ?? "loading"}</span>;
    }

    async function flush() {
      await act(async () => {
        for (let i = 0; i < 10; i += 1) await Promise.resolve();
      });
    }

    it("a window opened before the relay connects", async () => {
      const s = shell("connecting");
      render(
        <BackendProvider backend={s.backend}>
          <Relay />
        </BackendProvider>,
      );
      s.register();
      await flush();
      expect(screen.getByTestId("relay")).toHaveTextContent("connecting");
      act(() => {
        s.emit(relayEvent("authenticating"));
        s.emit(relayEvent("connected"));
      });
      expect(screen.getByTestId("relay")).toHaveTextContent("connected");
    });

    it("a window opened as the relay connects, before its listener is registered", async () => {
      const s = shell("authenticating");
      render(
        <BackendProvider backend={s.backend}>
          <Relay />
        </BackendProvider>,
      );
      await flush();
      // Connected while the window's listener is still on its way: the event cannot reach it.
      act(() => {
        s.emit(relayEvent("connected"));
      });
      s.register();
      await flush();
      expect(screen.getByTestId("relay")).toHaveTextContent("connected");
    });

    it("a window opened as the relay connects, while its first state is on its way", async () => {
      const s = shell("authenticating");
      s.hold();
      render(
        <BackendProvider backend={s.backend}>
          <Relay />
        </BackendProvider>,
      );
      s.register();
      await flush();
      // The state was taken before the relay connected; the event overtakes it.
      act(() => {
        s.emit(relayEvent("connected"));
      });
      s.answer();
      await flush();
      expect(screen.getByTestId("relay")).toHaveTextContent("connected");
    });

    it("a window opened after the relay connected", async () => {
      const s = shell("connected");
      render(
        <BackendProvider backend={s.backend}>
          <Relay />
        </BackendProvider>,
      );
      s.register();
      await flush();
      expect(screen.getByTestId("relay")).toHaveTextContent("connected");
    });

    it("a full state that comes after the first one replaces it, events after it still fold in", async () => {
      const s = shell("authenticating");
      s.hold();
      render(
        <BackendProvider backend={s.backend}>
          <Relay />
        </BackendProvider>,
      );
      s.register();
      await flush();
      act(() => {
        s.emit({ type: "state", ...new MockBackend().peek(), relay: relayAt("reconnecting") });
        s.emit(relayEvent("connected"));
      });
      // The full state shows at once; the first state's answer is older than both.
      expect(screen.getByTestId("relay")).toHaveTextContent("connected");
      s.answer();
      await flush();
      expect(screen.getByTestId("relay")).toHaveTextContent("connected");
    });
  });

  it("surfaces getState failures and guards hook usage", async () => {
    const backend = {
      getState: () => Promise.reject(new Error("core down")),
      invoke: () => Promise.resolve(),
      on: () => () => undefined,
      audioDevices: () => Promise.resolve([]),
      audioOutputs: () =>
        Promise.resolve({ system_audio: { state: "unsupported" as const }, devices: [] }),
      historyExport: () => Promise.resolve({ kind: "cancelled" as const }),
      importsState: () => Promise.resolve({ available: false, extensions: [], jobs: [] }),
      importPick: () => Promise.resolve([]),
      onFileDrop: () => () => undefined,
      backupExport: () => Promise.resolve({ kind: "cancelled" as const }),
      backupOpen: () => Promise.resolve({ kind: "cancelled" as const }),
      backupImport: () => Promise.resolve({ kind: "failed" as const, code: "expired" as const }),
      backupDiscard: () => Promise.resolve(),
      backupFolderChoose: () => Promise.resolve({ kind: "cancelled" as const }),
      backupFolderOpen: () => Promise.resolve(),
      meter: () => Promise.resolve(() => undefined),
      updateStatus: () => Promise.resolve({ state: "idle" as const }),
      vocabularyPreview: () =>
        Promise.resolve({ corrected: "", output: "", corrections: [], rules: [] }),
      vocabularyBacktest: () => Promise.resolve({ scanned: 0, matched: 0, kept: 0, examples: [] }),
      vocabularySuggestions: () => Promise.resolve({ status: "history_off" as const }),
      vocabularySuggestionAccept: () =>
        Promise.resolve({
          kind: "refused" as const,
          refusal: { kind: "failed" as const, message: "core down" },
        }),
      vocabularySuggestionUndo: () => Promise.resolve({ kind: "expired" as const }),
      vocabularySuggestionsAccept: () => Promise.resolve({ items: [], undo: null }),
      vocabularySuggestionsIgnore: () =>
        Promise.resolve({ kind: "failed" as const, message: "core down" }),
      vocabularySuggestionsUndo: () => Promise.resolve({ kind: "expired" as const }),
      rulesWarnings: () => Promise.resolve([]),
      rulesExport: () => Promise.resolve("version = 1\n"),
      recentApps: () => Promise.resolve([]),
      historyQuery: () => Promise.resolve({ entries: [], matching: 0, total: 0 }),
      historyEntry: () => Promise.resolve(null),
      historyStats: () =>
        Promise.resolve({
          buckets: [],
          total: { count: 0, raw_chars: 0, corrected_chars: 0, spoken_ms: 0, latency_ms: 0 },
        }),
      historyHits: () => Promise.resolve({ dictionary: {}, rules: {} }),
      mirrorHistoryQuery: () => Promise.resolve({ entries: [], matching: 0, total: 0 }),
      mirrorHistoryEntry: () => Promise.resolve(null),
      mirrorProfile: () => Promise.resolve(null),
      permissionsStatus: () => Promise.resolve(notApplicablePermissions("linux")),
      permissionsRequest: () => Promise.resolve(),
      injectPreflight: () => Promise.resolve(uncheckedPreflight("linux")),
      pasteText: () => Promise.resolve({ kind: "pasted" as const }),
      overlayCopy: () => Promise.resolve(),
      overlayPaste: () => Promise.resolve({ kind: "pasted" as const }),
      openHistory: () => Promise.resolve(),
      providerConsoleOpen: () => Promise.resolve(),
      projectLinkOpen: () => Promise.resolve(),
      guideOpen: () => Promise.resolve(),
      modelFolderOpen: () => Promise.resolve(),
      modelLinkOpen: () => Promise.resolve(),
      modelsLocationChoose: () => Promise.resolve(false),
      feedbackDiagnostics: () => Promise.reject(new Error("no feedback")),
      feedbackSubmit: () => Promise.reject(new Error("no feedback")),
      feedbackAttachmentAdd: () => Promise.reject(new Error("no feedback")),
      feedbackAttachmentRemove: () => Promise.resolve(),
      feedbackAttachmentsClear: () => Promise.resolve(),
      phoneClipboardRead: () => Promise.resolve(null),
      presetsBuiltin: () => Promise.resolve([]),
      scenesBuiltin: () => Promise.resolve([]),
    };
    render(
      <BackendProvider backend={backend}>
        <Probe />
      </BackendProvider>,
    );
    await waitFor(() => {
      expect(screen.getByTestId("error")).toHaveTextContent("core down");
    });
    const failing = { ...backend, getState: () => Promise.reject("plain") };
    render(
      <BackendProvider backend={failing}>
        <Probe />
      </BackendProvider>,
    );
    await waitFor(() => {
      expect(screen.getAllByTestId("error")[1]).toHaveTextContent("plain");
    });
    expect(() => renderHook(() => useBackend())).toThrow(
      "useBackend must be used inside <BackendProvider>",
    );
  });
});
