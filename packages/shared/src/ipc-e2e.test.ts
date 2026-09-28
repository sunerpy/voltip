// Real cross-language flow: this TypeScript drives two real Rust cores (desktop + phone) through
// `voltip-bridge-harness` (the shells' command surface over stdio) and a real local relay, using
// the production `TauriBackend` with a transport that speaks JSON lines instead of the webview
// IPC. No WebView, no fixed sleeps: every step waits for the state or event that proves it.
//
// Run with `pnpm --filter @voltip/shared run test:e2e` (or `make e2e-ipc`); it is not part of the
// unit test / coverage runs because it builds and spawns Rust binaries.
import { type ChildProcessWithoutNullStreams, execFileSync, spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { connect, createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { z } from "zod";

import { type UiEvent, type UiState, applyEvent, uiStateSchema } from "./schema";
import { type TauriTransport, TauriBackend } from "./tauri-backend";

const REPO_ROOT = fileURLToPath(new URL("../../../", import.meta.url));
const BUILD_TIMEOUT_MS = 600_000;
const STEP_TIMEOUT_MS = 30_000;
const PORT_POLL_MS = 100;
const TEXT_TO_PHONE = "把 fetchUser 改成 async";
const TEXT_TO_DESKTOP = "ok";

const responseSchema = z.union([
  z.object({ id: z.number(), ok: z.unknown() }),
  z.object({ id: z.number(), err: z.string() }),
]);
const lineSchema = z.union([z.object({ event: z.unknown() }), responseSchema]);

/** `TauriTransport` over a harness process: invoke = one request line, listen = event lines. */
class HarnessTransport implements TauriTransport {
  private nextId = 1;
  private readonly pending = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (error: Error) => void }
  >();
  private readonly listeners = new Set<(event: { payload: unknown }) => void>();
  readonly stderr: string[] = [];

  constructor(private readonly proc: ChildProcessWithoutNullStreams) {
    createInterface({ input: proc.stdout }).on("line", (line) => this.onLine(line));
    createInterface({ input: proc.stderr }).on("line", (line) => this.stderr.push(line));
    proc.on("exit", (code) => {
      for (const { reject } of this.pending.values()) {
        reject(new Error(`harness exited with ${code} while a request was pending`));
      }
      this.pending.clear();
    });
  }

  private onLine(line: string): void {
    const parsed = lineSchema.safeParse(JSON.parse(line));
    if (!parsed.success) throw new Error(`harness wrote an unknown line: ${line}`);
    const msg = parsed.data;
    if ("event" in msg) {
      for (const listener of this.listeners) listener({ payload: msg.event });
      return;
    }
    const waiter = this.pending.get(msg.id);
    this.pending.delete(msg.id);
    if (!waiter) throw new Error(`response for unknown request ${msg.id}`);
    if ("err" in msg) waiter.reject(new Error(msg.err));
    else waiter.resolve(msg.ok);
  }

  // Own properties (not prototype methods): `TauriBackend` spreads the transport object.
  readonly invoke = (command: string, args?: Record<string, unknown>): Promise<unknown> => {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.proc.stdin.write(`${JSON.stringify({ id, cmd: command, args })}\n`);
    });
  };

  readonly listen = (
    _event: string,
    handler: (event: { payload: unknown }) => void,
  ): Promise<() => void> => {
    this.listeners.add(handler);
    return Promise.resolve(() => {
      this.listeners.delete(handler);
    });
  };

  readonly warn = (message: string, detail: unknown): void => {
    throw new Error(`${message}: ${JSON.stringify(detail)}`);
  };
}

/** One device: a harness process, its `TauriBackend`, and the state folded from its events. */
class Device {
  readonly transport: HarnessTransport;
  readonly backend: TauriBackend;
  readonly dataDir: string;
  readonly events: UiEvent[] = [];
  state: UiState | undefined;
  private readonly waiters = new Set<() => void>();
  private snapshotInFlight: UiEvent[] | undefined;

  constructor(
    readonly name: string,
    harness: string,
    relayUrl: string,
  ) {
    this.dataDir = mkdtempSync(join(tmpdir(), `voltip-e2e-${name}-`));
    const proc = spawn(
      harness,
      [
        "--data-dir",
        this.dataDir,
        "--relay-url",
        relayUrl,
        "--device-name",
        name,
        "--direct-bind",
        "127.0.0.1:0",
      ],
      { stdio: "pipe" },
    );
    this.proc = proc;
    this.transport = new HarnessTransport(proc);
    this.backend = new TauriBackend(this.transport);
    this.backend.on((event) => this.onEvent(event));
  }
  private readonly proc: ChildProcessWithoutNullStreams;

  private onEvent(event: UiEvent): void {
    this.events.push(event);
    if (this.snapshotInFlight) this.snapshotInFlight.push(event);
    if (this.state) this.state = applyEvent(this.state, event);
    for (const wake of this.waiters) wake();
  }

  /** `core_state`, then re-fold whatever arrived while the request was in flight. */
  async start(): Promise<UiState> {
    this.snapshotInFlight = [];
    const snapshot = await this.backend.getState();
    const during = this.snapshotInFlight;
    this.snapshotInFlight = undefined;
    this.state = during.reduce(applyEvent, snapshot);
    for (const wake of this.waiters) wake();
    return this.state;
  }

  private wait<T>(label: string, probe: () => T | undefined): Promise<T> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiters.delete(check);
        reject(
          new Error(
            `${this.name}: ${label} not reached in ${STEP_TIMEOUT_MS} ms\nstate: ${JSON.stringify(this.state)}\nstderr: ${this.transport.stderr.join("\n")}`,
          ),
        );
      }, STEP_TIMEOUT_MS);
      const check = () => {
        const value = probe();
        if (value === undefined) return;
        clearTimeout(timer);
        this.waiters.delete(check);
        resolve(value);
      };
      this.waiters.add(check);
      check();
    });
  }

  waitState(label: string, pred: (state: UiState) => boolean): Promise<UiState> {
    return this.wait(label, () => (this.state && pred(this.state) ? this.state : undefined));
  }

  waitEvent<E extends UiEvent>(label: string, pick: (event: UiEvent) => event is E): Promise<E> {
    return this.wait(label, () => this.events.find(pick));
  }

  stop(): void {
    this.proc.stdin.end();
    this.proc.kill();
    rmSync(this.dataDir, { recursive: true, force: true });
  }
}

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      server.close(() => {
        if (address && typeof address === "object") resolve(address.port);
        else reject(new Error("no port"));
      });
    });
  });
}

function waitForPort(port: number): Promise<void> {
  const deadline = Date.now() + STEP_TIMEOUT_MS;
  return new Promise((resolve, reject) => {
    const attempt = () => {
      const socket = connect({ host: "127.0.0.1", port });
      socket.once("connect", () => {
        socket.end();
        resolve();
      });
      socket.once("error", () => {
        if (Date.now() > deadline) reject(new Error(`relay did not listen on ${port}`));
        else setTimeout(attempt, PORT_POLL_MS);
      });
    };
    attempt();
  });
}

/** Run `command` to completion without blocking the event loop: a synchronous build would keep
 *  the vitest worker from answering its runner for as long as cargo takes, and the runner gives up
 *  after 60 s ("Timeout calling onTaskUpdate", CI 2026-09-27) even though every test passed. */
function run(command: string, args: string[], timeoutMs: number): Promise<void> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: REPO_ROOT, stdio: "inherit", timeout: timeoutMs });
    child.on("error", reject);
    child.on("exit", (code, signal) => {
      if (code === 0) resolve();
      else reject(new Error(`${command} ${args.join(" ")} exited with ${code ?? signal}`));
    });
  });
}

function binaryPath(name: string): string {
  const metadata = z.object({ target_directory: z.string() }).parse(
    JSON.parse(
      execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
        cwd: REPO_ROOT,
        encoding: "utf8",
      }),
    ),
  );
  return join(
    metadata.target_directory,
    "debug",
    process.platform === "win32" ? `${name}.exe` : name,
  );
}

const isMessage = (e: UiEvent): e is Extract<UiEvent, { type: "message" }> => e.type === "message";
const isTrusted = (e: UiEvent): e is Extract<UiEvent, { type: "trusted" }> => e.type === "trusted";
const online = (s: UiState) => s.devices.some((d) => d.connection.state === "online");

describe("TypeScript ↔ Rust core over the bridge harness and a local relay", () => {
  let relay: ChildProcessWithoutNullStreams | undefined;
  let desktop: Device | undefined;
  let phone: Device | undefined;

  beforeAll(async () => {
    await run(
      "cargo",
      [
        "build",
        "-q",
        "-p",
        "voltip-tauri-bridge",
        "--bin",
        "voltip-bridge-harness",
        "-p",
        "voltip-relay",
      ],
      BUILD_TIMEOUT_MS,
    );
    const port = await freePort();
    relay = spawn(binaryPath("voltip-relay"), ["--bind", `127.0.0.1:${port}`], { stdio: "pipe" });
    await waitForPort(port);
    const relayUrl = `ws://127.0.0.1:${port}/ws`;
    const harness = binaryPath("voltip-bridge-harness");
    desktop = new Device("Surface-Laptop", harness, relayUrl);
    phone = new Device("Pixel 10", harness, relayUrl);
    await Promise.all([desktop.start(), phone.start()]);
  }, BUILD_TIMEOUT_MS);

  afterAll(() => {
    desktop?.stop();
    phone?.stop();
    relay?.kill();
  });

  it("pairs by code, verifies the safety code, comes online and exchanges E2EE text", async () => {
    if (!desktop || !phone) throw new Error("devices not started");
    const desk = await desktop.waitState("identity", (s) => s.identity !== null);
    expect(desk.identity?.name).toBe("Surface-Laptop");
    expect(desk.secret_backend).toBe("memory");
    expect(uiStateSchema.parse(desk)).toEqual(desk);
    await desktop.waitState("relay connected", (s) => s.relay.state === "connected");
    await phone.waitState("relay connected", (s) => s.relay.state === "connected");

    await desktop.backend.invoke("pairing_start");
    const waiting = await desktop.waitState(
      "waiting_for_peer",
      (s) => s.pairing.state.state === "waiting_for_peer" && s.pairing.code !== undefined,
    );
    const code = waiting.pairing.code;
    if (code === undefined) throw new Error("no pairing code");
    expect(code).toMatch(/^\d{3} \d{3}$/);
    expect(waiting.pairing.ticket_uri).toMatch(/^voltip:\/\/pair\?/);

    await phone.backend.invoke("pairing_join_code", { code });
    const verifyDesk = await desktop.waitState(
      "awaiting_verification",
      (s) => s.pairing.state.state === "awaiting_verification",
    );
    const verifyPhone = await phone.waitState(
      "awaiting_verification",
      (s) => s.pairing.state.state === "awaiting_verification",
    );
    expect(verifyDesk.pairing.safety_code).toBeDefined();
    expect(verifyPhone.pairing.safety_code).toEqual(verifyDesk.pairing.safety_code);
    expect(verifyDesk.pairing.safety_code?.words).toHaveLength(4);

    await desktop.backend.invoke("pairing_confirm");
    await phone.backend.invoke("pairing_confirm");
    const trustedOnDesk = await desktop.waitEvent("trusted event", isTrusted);
    const trustedOnPhone = await phone.waitEvent("trusted event", isTrusted);
    expect(trustedOnDesk.name).toBe("Pixel 10");
    expect(trustedOnPhone.name).toBe("Surface-Laptop");
    expect(trustedOnDesk.public_key).toBe(phone.state?.identity?.public_key);
    expect(trustedOnPhone.public_key).toBe(desktop.state?.identity?.public_key);
    await desktop.waitState("pairing trusted", (s) => s.pairing.state.state === "trusted");
    await phone.waitState("pairing trusted", (s) => s.pairing.state.state === "trusted");

    // Presence: both device lists show the other side online (devices events, not polling).
    const deskDevices = await desktop.waitState("phone online", online);
    const phoneDevices = await phone.waitState("desktop online", online);
    expect(deskDevices.devices.map((d) => d.device.name)).toEqual(["Pixel 10"]);
    expect(phoneDevices.devices.map((d) => d.device.name)).toEqual(["Surface-Laptop"]);

    await desktop.backend.invoke("send_text", {
      publicKey: trustedOnDesk.public_key,
      body: TEXT_TO_PHONE,
    });
    const received = await phone.waitEvent(
      "message on phone",
      (e): e is Extract<UiEvent, { type: "message" }> => isMessage(e) && e.body === TEXT_TO_PHONE,
    );
    expect(received.from).toBe(desktop.state?.identity?.public_key);

    await phone.backend.invoke("send_text", {
      publicKey: trustedOnPhone.public_key,
      body: TEXT_TO_DESKTOP,
    });
    const reply = await desktop.waitEvent(
      "message on desktop",
      (e): e is Extract<UiEvent, { type: "message" }> => isMessage(e) && e.body === TEXT_TO_DESKTOP,
    );
    expect(reply.from).toBe(phone.state?.identity?.public_key);

    // Argument validation errors travel back as rejected promises, like Tauri's `Err(String)`.
    await expect(desktop.backend.invoke("device_forget", { publicKey: "bad" })).rejects.toThrow(
      /64 hex/,
    );
    // Nothing the backends accepted was invalid: `warn` in the transport throws.
    expect(desktop.transport.stderr.filter((l) => l.includes("WARN"))).toEqual([]);
    expect(phone.transport.stderr.filter((l) => l.includes("WARN"))).toEqual([]);
  });
});
