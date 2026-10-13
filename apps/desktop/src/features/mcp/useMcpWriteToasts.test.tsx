import { type McpActivity, defaultMcpSettings } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";
import { MCP_TOAST_MS, routeForWrites } from "./useMcpWriteToasts";

function core() {
  return new MockBackend({ settings: { mcp: { ...defaultMcpSettings(), enabled: true } } });
}

const entry = (term: string) => ({ term, heard_as: [`${term}?`], enabled: true });
const rule = (name: string) => ({
  name,
  kind: "literal" as const,
  pattern: `${name}?`,
  replacement: name,
  case_sensitive: true,
  enabled: true,
});

function toasts(): string[] {
  const viewport = screen.queryByTestId("toast-viewport");
  return viewport === null
    ? []
    : within(viewport)
        .queryAllByRole("status")
        .map((toast) => toast.textContent ?? "");
}

describe("toasts for what the AI tools write", () => {
  it("a tool's entry is announced with 查看, which opens the dictionary; its next write joins the toast", async () => {
    const user = userEvent.setup();
    const backend = core();
    renderApp({ path: "/", backend });
    await screen.findByTestId("page-home");
    act(() => {
      backend.simulateMcpEntry("Claude Code", entry("Voltip"));
    });
    expect(await screen.findByText("Claude Code 添加了 1 个词条")).toBeInTheDocument();
    act(() => {
      backend.simulateMcpEntry("Claude Code", entry("Paseo"));
    });
    expect(await screen.findByText("Claude Code 添加了 2 个词条")).toBeInTheDocument();
    expect(toasts()).toEqual(["Claude Code 添加了 2 个词条查看"]);
    await user.click(screen.getByRole("button", { name: "查看" }));
    expect(await screen.findByTestId("page-dictionary")).toBeInTheDocument();
    expect(toasts()).toEqual([]);
  });

  it("a rule waiting for approval says so and 查看 opens the rules; another tool gets its own toast", async () => {
    const user = userEvent.setup();
    const backend = core();
    renderApp({ path: "/", backend });
    await screen.findByTestId("page-home");
    act(() => {
      backend.simulateMcpRule("Claude Code", rule("飞书"));
      backend.simulateMcpEntry("Codex", entry("Voltip"));
    });
    expect(await screen.findByText("Claude Code 新建了 1 条规则，确认后才会生效")).toBeVisible();
    expect(screen.getByText("Codex 添加了 1 个词条")).toBeVisible();
    // A mix of writes from one tool is counted as changes.
    act(() => {
      backend.simulateMcpEntry("Codex", entry("Paseo"));
      backend.simulateMcpRule("Codex", rule("推送"));
    });
    expect(await screen.findByText("Codex 修改了 3 处词典和规则")).toBeVisible();
    const toast = screen.getByText("Codex 修改了 3 处词典和规则").closest('[role="status"]');
    expect(toast).not.toBeNull();
    await user.click(within(toast as HTMLElement).getByRole("button", { name: "查看" }));
    expect(await screen.findByTestId("page-rules")).toBeInTheDocument();
  });

  it("reads, undone writes and the activity from before the window opened are not announced", async () => {
    const user = userEvent.setup();
    const backend = core();
    // Written before the window opened: listed in the settings, never announced.
    backend.simulateMcpEntry("Claude Code", entry("Voltip"));
    renderApp({ path: "/settings/mcp", backend });
    await screen.findByTestId("mcp-pane");
    act(() => {
      backend.simulateMcpRead("Claude Code", "read_summary", 12);
      backend.simulateMcpRead("Codex", "list_rules");
    });
    await user.click(
      screen.getByRole("button", { name: "撤销：Claude Code 添加了词条「Voltip」" }),
    );
    expect(await screen.findByText("已撤销 · Claude Code 添加了词条「Voltip」")).toBeVisible();
    expect(toasts()).toEqual(["已撤销 · Claude Code 添加了词条「Voltip」"]);
  });

  it("regression: the log the core reports after the window loaded is not announced as new writes", async () => {
    // The webview can ask for the state before the core's first MCP status: then the stored
    // activity arrives in an event, and only what comes after it is new.
    const backend = new MockBackend({
      mcp: { available: false, phase: "off", clients: [], activity: [] },
    });
    renderApp({ path: "/", backend });
    await screen.findByTestId("page-home");
    const old: McpActivity = {
      id: "00000000-0000-4000-8000-00000000c0de",
      at_ms: 1,
      client: "Claude Code",
      action: "add_entry",
      subject: "Voltip",
      pending: false,
      undoable: true,
      undone: false,
    };
    act(() => {
      backend.publish({ type: "mcp", available: true, phase: "off", clients: [], activity: [old] });
    });
    expect(toasts()).toEqual([]);
    act(() => {
      backend.simulateMcpEntry("Codex", entry("Paseo"));
    });
    expect(await screen.findByText("Codex 添加了 1 个词条")).toBeVisible();
    expect(toasts()).toEqual(["Codex 添加了 1 个词条查看"]);
  });

  it("a write after the toast went away starts a new count", async () => {
    const backend = core();
    renderApp({ path: "/", backend });
    await screen.findByTestId("page-home");
    act(() => {
      backend.simulateMcpEntry("Claude Code", entry("Voltip"));
    });
    expect(await screen.findByText("Claude Code 添加了 1 个词条")).toBeVisible();
    // The clock moves past the toast's time (its own timer still holds it on screen).
    const realNow = Date.now.bind(Date);
    const clock = vi.spyOn(Date, "now").mockImplementation(() => realNow() + MCP_TOAST_MS + 1);
    try {
      act(() => {
        backend.simulateMcpEntry("Claude Code", entry("Paseo"));
      });
      expect(await screen.findAllByText("Claude Code 添加了 1 个词条")).toHaveLength(2);
    } finally {
      clock.mockRestore();
    }
  });

  it("查看 opens the rules for a rule and the dictionary otherwise", () => {
    const write = (action: McpActivity["action"]): McpActivity => ({
      id: action,
      at_ms: 1,
      client: "Codex",
      action,
      pending: false,
      undoable: true,
      undone: false,
    });
    expect(routeForWrites([write("add_entry"), write("merge_entry")])).toEqual({
      name: "dictionary",
    });
    expect(routeForWrites([write("add_entry"), write("update_rule")])).toEqual({ name: "rules" });
    expect(routeForWrites([write("add_rule")])).toEqual({ name: "rules" });
  });
});
