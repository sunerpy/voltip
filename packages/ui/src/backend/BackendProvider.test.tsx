import { notApplicablePermissions, uncheckedPreflight } from "@voltip/shared";
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

  it("applies a full state event before core_state resolves and ignores partial ones", async () => {
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
      meter: inner.meter.bind(inner),
      updateStatus: inner.updateStatus.bind(inner),
      vocabularyPreview: inner.vocabularyPreview.bind(inner),
      rulesExport: inner.rulesExport.bind(inner),
      recentApps: inner.recentApps.bind(inner),
      historyQuery: inner.historyQuery.bind(inner),
      historyEntry: inner.historyEntry.bind(inner),
      historyStats: inner.historyStats.bind(inner),
      historyHits: inner.historyHits.bind(inner),
      permissionsStatus: inner.permissionsStatus.bind(inner),
      permissionsRequest: inner.permissionsRequest.bind(inner),
      injectPreflight: inner.injectPreflight.bind(inner),
      pasteText: inner.pasteText.bind(inner),
      providerConsoleOpen: inner.providerConsoleOpen.bind(inner),
      projectLinkOpen: inner.projectLinkOpen.bind(inner),
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

  it("surfaces getState failures and guards hook usage", async () => {
    const backend = {
      getState: () => Promise.reject(new Error("core down")),
      invoke: () => Promise.resolve(),
      on: () => () => undefined,
      audioDevices: () => Promise.resolve([]),
      meter: () => Promise.resolve(() => undefined),
      updateStatus: () => Promise.resolve({ state: "idle" as const }),
      vocabularyPreview: () =>
        Promise.resolve({ corrected: "", output: "", corrections: [], rules: [] }),
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
      permissionsStatus: () => Promise.resolve(notApplicablePermissions("linux")),
      permissionsRequest: () => Promise.resolve(),
      injectPreflight: () => Promise.resolve(uncheckedPreflight("linux")),
      pasteText: () => Promise.resolve({ kind: "pasted" as const }),
      providerConsoleOpen: () => Promise.resolve(),
      projectLinkOpen: () => Promise.resolve(),
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
