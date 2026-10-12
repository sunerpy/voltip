// The local MCP service's helpers and the preview's service (docs/dictation.md §25): the setup
// commands the settings show are the core's (fixtures/ipc/mcp-setup.json), the history page's diff
// is the one the core's correction summary computes (fixtures/ipc/diff.json), the activity reads as
// text in both languages, and the preview's service behaves as the core's: its own port, the levels
// applied at once, rules from an AI tool waiting for approval, undo while nothing changed since.
import { readFileSync } from "node:fs";
import { z } from "zod";

import {
  MCP_SETUP_LABELS,
  isMcpWrite,
  mcpActivityText,
  mcpAddress,
  mcpRangeText,
  mcpSetupCommand,
  mcpWritesToast,
  pendingRules,
  pendingRulesClient,
} from "./mcp";
import { MCP_UNAVAILABLE, MOCK_TAKEN_PORTS, MockBackend } from "./mock-backend";
import { diffSegments } from "./refine";
import {
  MCP_ACTIONS,
  type McpActivity,
  type ReplacementRule,
  type UiEvent,
  defaultSettings,
  mcpSetupClientSchema,
  uiStateSchema,
} from "./schema";
import { exportRulesToml, previewVocabulary } from "./vocabulary";

const FIXTURES = new URL("./fixtures/ipc/", import.meta.url);

function loadFixture(name: string): unknown {
  return JSON.parse(readFileSync(new URL(name, FIXTURES), "utf8"));
}

function collect(backend: MockBackend) {
  const events: UiEvent[] = [];
  backend.on((e) => events.push(e));
  return events;
}

function activity(partial: Partial<McpActivity> & Pick<McpActivity, "action">): McpActivity {
  return {
    id: "a",
    at_ms: 1,
    client: "Claude Code",
    pending: false,
    undoable: false,
    undone: false,
    ...partial,
  };
}

function rule(partial: Partial<ReplacementRule> = {}): ReplacementRule {
  return {
    id: "00000000-0000-4000-9000-000000000001",
    name: "非书 → 飞书",
    kind: "literal",
    pattern: "非书",
    replacement: "飞书",
    case_sensitive: true,
    enabled: true,
    created_at_ms: 1,
    updated_at_ms: 1,
    source: { kind: "manual" },
    pending: false,
    ...partial,
  };
}

describe("the setup commands", () => {
  it("are the ones the core copies, for every tool and both shells", () => {
    const rows = z
      .array(
        z.object({
          client: mcpSetupClientSchema,
          windows: z.boolean(),
          address: z.string(),
          token: z.string(),
          command: z.string(),
        }),
      )
      .parse(loadFixture("mcp-setup.json"));
    expect(rows).toHaveLength(4);
    for (const row of rows) {
      expect(mcpSetupCommand(row.client, row.address, row.token, row.windows)).toBe(row.command);
    }
    expect(mcpAddress(47841)).toBe("http://127.0.0.1:47841/mcp");
    expect(MCP_SETUP_LABELS).toEqual({ claude_code: "Claude Code", codex: "Codex" });
    expect(mcpSetupCommand("claude_code", mcpAddress(47841), "<令牌>", false)).toBe(
      'claude mcp add --transport http voltip http://127.0.0.1:47841/mcp --header "Authorization: Bearer <令牌>"',
    );
  });
});

describe("the history page's diff", () => {
  it("is the one the core's correction summary computes", () => {
    const rows = z
      .array(
        z.object({
          raw: z.string(),
          text: z.string(),
          segments: z.array(z.object({ kind: z.enum(["same", "add", "del"]), text: z.string() })),
        }),
      )
      .parse(loadFixture("diff.json"));
    expect(rows.length).toBeGreaterThan(5);
    for (const row of rows) {
      expect(diffSegments({ rawText: row.raw, text: row.text })).toEqual(row.segments);
    }
  });
});

describe("the activity as text", () => {
  it("names who did what, about what and how many, in both languages", () => {
    expect(mcpActivityText(activity({ action: "add_entry", subject: "Voltip" }))).toBe(
      "Claude Code 添加了词条「Voltip」",
    );
    expect(mcpActivityText(activity({ action: "add_entry", subject: "Voltip" }), "en")).toBe(
      "Claude Code added the entry 「Voltip」",
    );
    const waiting = activity({ action: "add_rule", subject: "非书 → 飞书", pending: true });
    expect(mcpActivityText(waiting)).toBe("Claude Code 新建了规则「非书 → 飞书」，待确认");
    expect(mcpActivityText(waiting, "en")).toBe(
      "Claude Code created the rule 「非书 → 飞书」, needs approval",
    );
    const read = activity({
      action: "search_history",
      client: "Codex",
      count: 12,
      range: "last_30_days",
    });
    expect(mcpActivityText(read)).toBe("Codex 读取了完整历史（最近 30 天，12 条）");
    expect(mcpActivityText(read, "en")).toBe(
      "Codex read the full history (the last 30 days, 12 entries)",
    );
    expect(mcpActivityText(activity({ action: "read_summary", count: 1 }), "en")).toBe(
      "Claude Code read the correction summary (1 pair)",
    );
    expect(
      mcpActivityText(activity({ action: "merge_entry", subject: "Voltip", count: 2 }), "en"),
    ).toBe("Claude Code added 2 heard-as forms to 「Voltip」");
    expect(mcpRangeText("all", "en")).toBe("all entries");
    for (const action of MCP_ACTIONS) {
      const zh = mcpActivityText(activity({ action, subject: "x", count: 2, range: "all" }));
      expect(zh.startsWith("Claude Code ")).toBe(true);
      expect(zh).not.toContain("{");
    }
    expect(MCP_ACTIONS.filter((action) => isMcpWrite({ action }))).toEqual([
      "add_entry",
      "merge_entry",
      "update_entry",
      "add_rule",
      "update_rule",
    ]);
  });

  it("the toast after an AI tool's writes says what it changed", () => {
    const added = [activity({ action: "add_entry" }), activity({ action: "add_entry" })];
    expect(mcpWritesToast("Claude Code", added)).toBe("Claude Code 添加了 2 个词条");
    expect(mcpWritesToast("Claude Code", added.slice(1), "en")).toBe(
      "Claude Code added 1 dictionary entry",
    );
    expect(mcpWritesToast("Codex", [activity({ action: "add_rule", pending: true })])).toBe(
      "Codex 新建了 1 条规则，确认后才会生效",
    );
    expect(mcpWritesToast("Codex", [activity({ action: "add_rule" })], "en")).toBe(
      "Codex made 1 change to the dictionary and rules",
    );
  });
});

describe("rules waiting for approval", () => {
  it("are listed with the tool that made them, and run nowhere", () => {
    const waiting = rule({
      pending: true,
      enabled: false,
      source: { kind: "mcp", client: "Codex" },
    });
    const manual = rule({ id: "00000000-0000-4000-9000-000000000002", name: "git push" });
    expect(pendingRules([manual, waiting])).toEqual([waiting]);
    expect(pendingRulesClient([manual, waiting])).toBe("Codex");
    const other = rule({
      id: "00000000-0000-4000-9000-000000000003",
      name: "b",
      pending: true,
      source: { kind: "mcp", client: "Claude Code" },
    });
    expect(pendingRulesClient([waiting, other])).toBeUndefined();
    expect(pendingRulesClient([manual])).toBeUndefined();
    // Even a file that says it is on: waiting runs nowhere, and exports as off.
    const forced = { ...waiting, enabled: true };
    expect(previewVocabulary([], [forced], "发到非书群").output).toBe("发到非书群");
    expect(exportRulesToml([forced])).toContain("enabled = false");
    expect(previewVocabulary([], [manual], "发到非书群").output).toBe("发到飞书群");
  });

  it("an old rules list without the fields reads as the user's, approved", () => {
    const state = uiStateSchema.parse({
      identity: null,
      settings: defaultSettings(),
      secret_backend: "memory",
      relay: { state: "disconnected", attempts: 0, source: "none" },
      pairing: { state: { state: "idle" }, local_confirmed: false, peer_confirmed: false },
      devices: [],
      rules: [
        {
          id: "r",
          name: "n",
          kind: "literal",
          pattern: "p",
          case_sensitive: true,
          enabled: true,
          created_at_ms: 1,
          updated_at_ms: 1,
        },
      ],
    });
    expect(state.rules[0]?.source).toEqual({ kind: "manual" });
    expect(state.rules[0]?.pending).toBe(false);
    expect(state.mcp).toEqual({ available: false, phase: "off", clients: [], activity: [] });
    expect(state.settings.mcp.port).toBe(47841);
  });
});

describe("MockBackend's MCP service (docs/dictation.md section 25)", () => {
  it("starts on its own port, applies the levels at once and reports a taken port", async () => {
    const backend = new MockBackend();
    const events = collect(backend);
    const phases = () => events.flatMap((e) => (e.type === "mcp" ? [e.phase] : []));
    const set = (patch: Partial<{ enabled: boolean; port: number }> = {}) =>
      backend.invoke("settings_set_mcp", {
        enabled: true,
        port: 47841,
        read: "summary",
        historyRange: "last_30_days",
        write: "dictionary",
        ...patch,
      });
    await set();
    expect(phases()).toEqual(["starting", "running"]);
    const state = await backend.getState();
    expect(state.mcp.address).toBe("http://127.0.0.1:47841/mcp");
    await backend.invoke("settings_set_mcp", {
      enabled: true,
      port: 47841,
      read: "full",
      historyRange: "last_7_days",
      write: "enable_rules",
    });
    // The levels need no restart.
    expect(phases()).toEqual(["starting", "running"]);
    expect((await backend.getState()).settings.mcp).toEqual({
      enabled: true,
      port: 47841,
      read: "full",
      history_range: "last_7_days",
      write: "enable_rules",
    });
    await set({ port: MOCK_TAKEN_PORTS[0] ?? 47999 });
    expect(phases().slice(-2)).toEqual(["starting", "failed"]);
    await set({ port: 80 });
    await set({ port: 47840 });
    const errors = events.flatMap((e) => (e.type === "error" ? [e.message] : []));
    expect(errors).toEqual([
      "mcp: 端口须在 1024–65535 之间",
      "mcp: 端口 47840 已用于本机服务，请换一个端口",
    ]);
    await set({ enabled: false });
    expect(phases().at(-1)).toBe("off");
    await backend.invoke("mcp_copy_token");
    await backend.invoke("mcp_rotate_token");
    await backend.invoke("mcp_copy_setup", { client: "codex" });
    expect([backend.mcpTokenCopies, backend.mcpTokenRotations, backend.mcpSetupCopies]).toEqual([
      1,
      1,
      ["codex"],
    ]);
    backend.destroy();
  });

  it("an AI tool's entry and waiting rule show up, approve, and undo while unchanged", async () => {
    const backend = new MockBackend({
      settings: { mcp: { ...defaultSettings().mcp, enabled: true } },
    });
    const events = collect(backend);
    const id = backend.simulateMcpEntry("Claude Code", {
      term: "Voltip",
      heard_as: ["维他命"],
      enabled: true,
    });
    const ruleId = backend.simulateMcpRule("Claude Code", {
      name: "非书 → 飞书",
      kind: "literal",
      pattern: "非书",
      replacement: "飞书",
      case_sensitive: true,
      enabled: true,
    });
    backend.simulateMcpRead("Codex", "search_history", 3);
    let state = await backend.getState();
    expect(state.dictionary[0]?.source).toEqual({ kind: "mcp", client: "Claude Code" });
    expect(state.rules[0]).toMatchObject({ id: ruleId, pending: true, enabled: false });
    expect(state.mcp.clients.map((c) => c.name)).toEqual(["Codex", "Claude Code"]);
    expect(state.mcp.activity.map((a) => [a.action, a.undoable, a.pending, a.range])).toEqual([
      ["search_history", false, false, "last_30_days"],
      ["add_rule", true, true, undefined],
      ["add_entry", true, false, undefined],
    ]);
    // Undo the entry while nobody changed it.
    const entryActivity = state.mcp.activity[2];
    await backend.invoke("mcp_undo", { id: entryActivity?.id ?? "" });
    state = await backend.getState();
    expect(state.dictionary.some((e) => e.id === id)).toBe(false);
    expect(state.mcp.activity[2]).toMatchObject({ undone: true, undoable: false });
    await backend.invoke("mcp_undo", { id: entryActivity?.id ?? "" });
    // The user approves the rule; its undo is refused from then on.
    await backend.invoke("rules_update", {
      id: ruleId,
      rule: {
        name: "非书 → 飞书",
        kind: "literal",
        pattern: "非书",
        replacement: "飞书",
        case_sensitive: true,
        enabled: true,
      },
    });
    state = await backend.getState();
    expect(state.rules[0]).toMatchObject({
      pending: false,
      enabled: true,
      source: { kind: "mcp", client: "Claude Code" },
    });
    await backend.invoke("mcp_undo", { id: state.mcp.activity[1]?.id ?? "" });
    await backend.invoke("mcp_undo", { id: state.mcp.activity[0]?.id ?? "" });
    await backend.invoke("mcp_undo", { id: "00000000-0000-4000-9000-00000000dead" });
    expect(events.flatMap((e) => (e.type === "error" ? [e.message] : []))).toEqual([
      "mcp: 这条活动已经撤销",
      "mcp: 无法撤销：这条规则之后又被修改过",
      "mcp: 这条活动不能撤销",
      "mcp: 没有这条活动记录",
    ]);
    backend.destroy();
  });

  it("an AI tool's rule is on at once when the settings allow it, and undoing it removes it", async () => {
    const backend = new MockBackend({
      settings: { mcp: { ...defaultSettings().mcp, enabled: true, write: "enable_rules" } },
    });
    const id = backend.simulateMcpRule("Codex", {
      name: "a",
      kind: "literal",
      pattern: "x",
      replacement: "y",
      case_sensitive: true,
      enabled: true,
    });
    let state = await backend.getState();
    expect(state.rules[0]).toMatchObject({ id, pending: false, enabled: true });
    await backend.invoke("mcp_undo", { id: state.mcp.activity[0]?.id ?? "" });
    state = await backend.getState();
    expect(state.rules).toEqual([]);
    backend.destroy();
  });

  it("the phone has no MCP service", async () => {
    const backend = new MockBackend({ role: "phone" });
    expect((await backend.getState()).mcp).toEqual({
      available: false,
      phase: "off",
      clients: [],
      activity: [],
    });
    await expect(backend.invoke("mcp_copy_token")).rejects.toThrow(MCP_UNAVAILABLE);
    await expect(backend.invoke("mcp_rotate_token")).rejects.toThrow(MCP_UNAVAILABLE);
    await expect(backend.invoke("mcp_copy_setup", { client: "claude_code" })).rejects.toThrow(
      MCP_UNAVAILABLE,
    );
    await expect(
      backend.invoke("settings_set_mcp", {
        enabled: true,
        port: 47841,
        read: "rules",
        historyRange: "all",
        write: "read_only",
      }),
    ).rejects.toThrow(MCP_UNAVAILABLE);
    await expect(
      backend.invoke("mcp_undo", { id: "00000000-0000-4000-9000-000000000001" }),
    ).rejects.toThrow(MCP_UNAVAILABLE);
    backend.destroy();
  });
});
