// Platform queries (docs/dictation.md §15): the wire schemas mirror `voltip_platform`, the gate
// mirrors `onboarding_gate` row for row, and both backends answer the three queries.
import {
  MockBackend,
  desktopIdentity,
  hostOsOf,
  mockPermissions,
  phoneIdentity,
} from "./mock-backend";
import {
  PERMISSIONS,
  PERMISSION_POLL_INTERVAL_MS,
  PERMISSION_POLL_MAX_ERRORS,
  type PermissionReport,
  type PermissionState,
  QUERY_COMMANDS,
  injectPreflightSchema,
  notApplicablePermissions,
  nothingToGrant,
  onboardingGate,
  permissionReportSchema,
  uncheckedPreflight,
} from "./schema";
import { TauriBackend } from "./tauri-backend";

const report = (microphone: PermissionState, accessibility: PermissionState): PermissionReport => ({
  platform: "macos",
  microphone,
  accessibility,
});

describe("permission wire (voltip_platform::PermissionReport)", () => {
  it("parses the snake_case report and refuses unknown states", () => {
    const wire = {
      platform: "macos",
      microphone: "granted",
      accessibility: "denied",
    };
    expect(permissionReportSchema.parse(wire)).toEqual(wire);
    expect(permissionReportSchema.safeParse({ ...wire, microphone: "maybe" }).success).toBe(false);
    expect(permissionReportSchema.safeParse({ ...wire, platform: "beos" }).success).toBe(false);
    expect(PERMISSIONS).toEqual(["microphone", "accessibility"]);
    // Regression (public release, 2026-09-27): no Input Monitoring — no trigger needs it.
    expect(permissionReportSchema.parse({ ...wire, input_monitoring: "granted" })).toEqual(wire);
    // voltip_platform::PollPlan::DEFAULT
    expect([PERMISSION_POLL_INTERVAL_MS, PERMISSION_POLL_MAX_ERRORS]).toEqual([1000, 3]);
  });

  it("onboarding gate matches the Rust onboarding_gate table", () => {
    const table: [PermissionReport, string[]][] = [
      [report("not_applicable", "not_applicable"), []],
      [report("granted", "granted"), []],
      [report("not_determined", "not_applicable"), []],
      [report("denied", "not_applicable"), ["microphone"]],
      [report("granted", "denied"), ["accessibility"]],
      [report("granted", "not_determined"), ["accessibility"]],
      [report("denied", "denied"), ["microphone", "accessibility"]],
      [report("not_determined", "not_determined"), ["accessibility"]],
    ];
    for (const [r, blocked] of table) expect(onboardingGate(r)).toEqual(blocked);
  });

  it("nothing to grant only when every permission is not applicable", () => {
    expect(nothingToGrant(notApplicablePermissions("linux"))).toBe(true);
    expect(nothingToGrant(report("granted", "not_applicable"))).toBe(false);
    expect(notApplicablePermissions("other").platform).toBe("other");
  });
});

describe("inject preflight wire (voltip_platform::InjectPreflight)", () => {
  it("parses a checked Windows answer and the unchecked answer of other hosts", () => {
    const elevated = {
      platform: "windows",
      checked: true,
      decision: "elevated_target",
      target_process: "regedit.exe",
      self_level: "medium",
      target_level: "high",
    };
    expect(injectPreflightSchema.parse(elevated)).toEqual(elevated);
    expect(injectPreflightSchema.parse(uncheckedPreflight("macos"))).toEqual({
      platform: "macos",
      checked: false,
      decision: "proceed",
      target_process: null,
      self_level: null,
      target_level: null,
    });
    expect(injectPreflightSchema.safeParse({ ...elevated, decision: "maybe" }).success).toBe(false);
    expect(injectPreflightSchema.safeParse({ ...elevated, target_level: "root" }).success).toBe(
      false,
    );
  });

  it("the three platform queries are queries, not UiCommands", () => {
    for (const q of ["permissions_status", "permissions_request", "inject_preflight"] as const)
      expect(QUERY_COMMANDS).toContain(q);
  });
});

describe("MockBackend platform queries", () => {
  it("defaults to the settled happy path of the identity's host", async () => {
    expect(hostOsOf("macos")).toBe("macos");
    expect(hostOsOf("windows")).toBe("windows");
    expect(hostOsOf("linux")).toBe("linux");
    expect(hostOsOf("android")).toBe("other");
    expect(hostOsOf("ios")).toBe("other");
    expect(mockPermissions("macos")).toEqual({
      platform: "macos",
      microphone: "granted",
      accessibility: "granted",
    });
    expect(mockPermissions("windows")).toEqual({
      ...notApplicablePermissions("windows"),
      microphone: "granted",
    });
    expect(mockPermissions("linux")).toEqual(notApplicablePermissions("linux"));
    // desktopIdentity() is a Windows machine; the phone gates nothing here.
    expect(await new MockBackend().permissionsStatus()).toEqual(mockPermissions("windows"));
    expect(await new MockBackend({ role: "phone" }).permissionsStatus()).toEqual(
      notApplicablePermissions("other"),
    );
    expect(await new MockBackend({ identity: phoneIdentity() }).injectPreflight()).toEqual(
      uncheckedPreflight("other"),
    );
    expect(await new MockBackend().injectPreflight()).toEqual(uncheckedPreflight("windows"));
  });

  it("a request grants a seeded permission from the next read, never an inapplicable one", async () => {
    const seeded = report("denied", "not_determined");
    const mock = new MockBackend({
      identity: { ...desktopIdentity(), platform: "macos" },
      permissions: seeded,
    });
    await mock.permissionsRequest("accessibility");
    expect(mock.permissionRequests).toEqual(["accessibility"]);
    expect(await mock.permissionsStatus()).toEqual({ ...seeded, accessibility: "granted" });
    // The seed itself is never mutated; reads are copies.
    expect(seeded.accessibility).toBe("not_determined");
    const a = await mock.permissionsStatus();
    a.microphone = "granted";
    expect((await mock.permissionsStatus()).microphone).toBe("denied");
  });

  it("a probe answers every read and may fail; setPermissions swaps it", async () => {
    let calls = 0;
    const mock = new MockBackend({
      permissions: () => {
        calls += 1;
        if (calls === 2) throw new Error("tcc unavailable");
        if (calls === 3) throw "plain failure";
        return notApplicablePermissions("linux");
      },
    });
    expect(await mock.permissionsStatus()).toEqual(notApplicablePermissions("linux"));
    await expect(mock.permissionsStatus()).rejects.toThrow("tcc unavailable");
    await expect(mock.permissionsStatus()).rejects.toThrow("plain failure");
    // A request against a probe is recorded but cannot change what the probe answers.
    await mock.permissionsRequest("microphone");
    expect(mock.permissionRequests).toEqual(["microphone"]);
    mock.setPermissions(report("granted", "granted"));
    expect(await mock.permissionsStatus()).toEqual(report("granted", "granted"));
    const preflight = {
      ...uncheckedPreflight("windows"),
      checked: true,
      decision: "unknown" as const,
    };
    expect(await new MockBackend({ injectPreflight: preflight }).injectPreflight()).toEqual(
      preflight,
    );
  });
});

describe("TauriBackend platform queries", () => {
  it("invokes the three commands and validates what comes back", async () => {
    const calls: { command: string; args: unknown }[] = [];
    const answers: Record<string, unknown> = {
      permissions_status: notApplicablePermissions("linux"),
      permissions_request: null,
      inject_preflight: uncheckedPreflight("linux"),
    };
    const backend = new TauriBackend({
      invoke: (command, args) => {
        calls.push({ command, args });
        return Promise.resolve(answers[command]);
      },
      listen: () => Promise.resolve(() => undefined),
    });
    expect(await backend.permissionsStatus()).toEqual(notApplicablePermissions("linux"));
    await backend.permissionsRequest("accessibility");
    expect(await backend.injectPreflight()).toEqual(uncheckedPreflight("linux"));
    expect(calls).toEqual([
      { command: "permissions_status", args: undefined },
      { command: "permissions_request", args: { permission: "accessibility" } },
      { command: "inject_preflight", args: undefined },
    ]);
    const broken = new TauriBackend({
      invoke: () => Promise.resolve({ platform: "linux", microphone: "sometimes" }),
      listen: () => Promise.resolve(() => undefined),
    });
    await expect(broken.permissionsStatus()).rejects.toThrow(/invalid|expected/i);
    await expect(broken.injectPreflight()).rejects.toThrow(/invalid|expected/i);
  });
});
