import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  MOCK_AVAILABLE_VERSION,
  MOCK_UPDATE_CHECK_MS,
  MOCK_UPDATE_TICK_MS,
  MockBackend,
  sampleDevices,
} from "@voltip/shared/mock";
import { zhT } from "@voltip/shared";
import { renderApp } from "../../test/render";
import { etaText, formatBytes, rateFrom } from "./download-rate";
import { ReleaseNotes, inlineParts, parseNotes } from "./release-notes";

function backend() {
  return new MockBackend({ devices: sampleDevices(1_758_700_000), now: () => 1_758_700_000_000 });
}

describe("release notes", () => {
  it("reads release-please's markdown as headings, lists and paragraphs, minus the version line", () => {
    const notes = [
      "## 0.0.2 (2026-09-25)",
      "",
      "### Features",
      "",
      "* **engines:** faster ([a1b2c3d](https://example.test/c/a1b2c3d))",
      "- a second item",
      "",
      "A paragraph",
      "that wraps.",
    ].join("\r\n");
    expect(parseNotes(notes, "0.0.2")).toEqual([
      { kind: "heading", level: 3, text: "Features" },
      {
        kind: "list",
        items: ["**engines:** faster ([a1b2c3d](https://example.test/c/a1b2c3d))", "a second item"],
      },
      { kind: "paragraph", text: "A paragraph that wraps." },
    ]);
    // Another version's heading stays; so does everything without a version.
    expect(parseNotes("## [0.0.3](x)", "0.0.2")[0]).toMatchObject({ kind: "heading" });
    expect(parseNotes("# v0.0.2", "0.0.2")).toEqual([]);
    expect(parseNotes("text\n* item\nmore")).toEqual([
      { kind: "paragraph", text: "text" },
      { kind: "list", items: ["item"] },
      { kind: "paragraph", text: "more" },
    ]);
  });

  it("regression: inline markup is text, a link is only its text, and nothing is HTML", () => {
    expect(inlineParts("**a:** b `c` ([d1](https://x.test)) [e](https://y.test)")).toEqual([
      { kind: "bold", text: "a:" },
      { kind: "text", text: " b " },
      { kind: "code", text: "c" },
      { kind: "text", text: " (d1) e" },
    ]);
    expect(inlineParts("see ([](https://x.test))")).toEqual([{ kind: "text", text: "see" }]);
    const { container } = render(
      <ReleaseNotes markdown={'* <img src=x onerror="alert(1)"> [link](javascript:alert(1))'} />,
    );
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("a")).toBeNull();
    expect(container.textContent).toBe('<img src=x onerror="alert(1)"> link');
  });
});

describe("download rate", () => {
  it("measures the speed over the window and words the time left", () => {
    expect(rateFrom([])).toBeUndefined();
    expect(rateFrom([{ at: 0, received: 0 }])).toBeUndefined();
    expect(
      rateFrom([
        { at: 0, received: 0 },
        { at: 300, received: 10 },
      ]),
    ).toBeUndefined();
    expect(
      rateFrom([
        { at: 0, received: 0 },
        { at: 1000, received: 2_097_152 },
      ]),
    ).toBe(2_097_152);
    expect(
      rateFrom([
        { at: 0, received: 5 },
        { at: 1000, received: 1 },
      ]),
    ).toBeUndefined();
    expect(formatBytes(3_145_728)).toBe("3.0 MB");
    expect(formatBytes(2048)).toBe("2 KB");
    const { t } = zhT;
    expect(etaText(0, 1000, 100, t)).toBe("剩余约 10 秒");
    expect(etaText(0, 100_000, 1000, t)).toBe("剩余约 1 分 40 秒");
    expect(etaText(10, 10, 5, t)).toBeUndefined();
    expect(etaText(0, undefined, 5, t)).toBeUndefined();
    expect(etaText(0, 10, undefined, t)).toBeUndefined();
  });
});

describe("UpdateDialog", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("regression: a new version shows on the title bar and opens a dialog with its notes, the download's speed and the restart", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const core = backend();
    renderApp({ backend: core });
    const user = userEvent.setup({ advanceTimers: (ms) => vi.advanceTimersByTime(ms) });
    await screen.findByRole("heading", { name: "首页", level: 1 });
    expect(screen.queryByTestId("update-badge")).toBeNull();
    // A check finds the new version: the title bar says so.
    await act(async () => {
      await core.invoke("update_check");
      vi.advanceTimersByTime(MOCK_UPDATE_CHECK_MS);
    });
    const badge = await screen.findByTestId("update-badge");
    expect(badge).toHaveTextContent(`新版本 ${MOCK_AVAILABLE_VERSION}`);
    await user.click(badge);
    const dialog = screen.getByRole("dialog", { name: `发现新版本 ${MOCK_AVAILABLE_VERSION}` });
    expect(within(dialog).getByTestId("update-current")).toHaveTextContent("当前 0.0.1");
    expect(within(dialog).getByTestId("update-published")).toHaveTextContent("发布于");
    const notes = within(dialog).getByTestId("release-notes");
    expect(within(notes).getByRole("heading", { name: "Features" })).toBeInTheDocument();
    expect(notes).toHaveTextContent("engines: a faster recognition path (a1b2c3d)");
    expect(notes).not.toHaveTextContent("0.0.2 (2026-09-25)");
    expect(notes.querySelector("a")).toBeNull();
    // The release page opens through the shell, which builds the URL.
    await user.click(within(dialog).getByRole("button", { name: "查看发布页" }));
    expect(core.linksOpened).toEqual(["releases"]);
    // 立即更新: the download, with its progress and, after a second reading, its speed.
    await user.click(within(dialog).getByRole("button", { name: "立即更新" }));
    await waitFor(() => {
      expect(screen.getByTestId("update-dialog")).toHaveAttribute("data-state", "downloading");
    });
    expect(screen.getByTestId("update-badge")).toHaveTextContent("下载中 33%");
    act(() => {
      vi.advanceTimersByTime(MOCK_UPDATE_TICK_MS * 3);
    });
    await waitFor(() => {
      expect(screen.getByTestId("update-dialog")).toHaveAttribute("data-state", "ready");
    });
    expect(screen.getByRole("dialog")).toHaveTextContent("已下载并检查完毕");
    expect(screen.getByTestId("update-badge")).toHaveTextContent("重启以更新");
    await user.click(screen.getByRole("button", { name: "重启并更新" }));
    await waitFor(() => {
      expect(screen.getByTestId("update-dialog")).toHaveAttribute("data-state", "installing");
    });
    expect(screen.queryByTestId("update-badge")).toBeNull();
  });

  it("shows the download's speed and time left once it has two readings; closing keeps it going", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const core = backend();
    renderApp({ backend: core });
    const user = userEvent.setup({ advanceTimers: (ms) => vi.advanceTimersByTime(ms) });
    await screen.findByRole("heading", { name: "首页", level: 1 });
    act(() => {
      core.simulateUpdate({
        state: "downloading",
        version: "0.0.2",
        received: 0,
        total: 50_000_000,
      });
    });
    await user.click(await screen.findByTestId("update-badge"));
    expect(screen.getByTestId("update-progress")).toHaveTextContent("0%");
    expect(screen.queryByTestId("update-speed")).toBeNull();
    act(() => {
      vi.advanceTimersByTime(1000);
      core.simulateUpdate({
        state: "downloading",
        version: "0.0.2",
        received: 10_485_760,
        total: 50_000_000,
      });
    });
    await waitFor(() => {
      expect(screen.getByTestId("update-speed")).toHaveTextContent(/MB\/s$/);
    });
    expect(screen.getByTestId("update-eta")).toHaveTextContent(/^剩余约/);
    await user.click(screen.getByRole("button", { name: "后台下载" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByTestId("update-badge")).toHaveTextContent("下载中 21%");
  });

  it("a failed update says why and retries; the settings row opens the same dialog", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const core = backend();
    renderApp({ backend: core, path: "/settings/general" });
    const user = userEvent.setup({ advanceTimers: (ms) => vi.advanceTimersByTime(ms) });
    await screen.findByRole("dialog", { name: "设置" });
    act(() => {
      core.simulateUpdate({
        state: "available",
        version: "0.0.2",
        current: "0.0.1",
      });
    });
    await user.click(await screen.findByRole("button", { name: "查看新版本" }));
    const dialog = screen.getByRole("dialog", { name: "发现新版本 0.0.2" });
    expect(within(dialog).getByText("这个版本没有写更新说明。")).toBeInTheDocument();
    act(() => {
      core.simulateUpdate({ state: "failed", message: "signature mismatch" });
    });
    const failed = screen.getByRole("dialog", { name: "更新失败" });
    expect(within(failed).getByRole("alert")).toHaveTextContent("更新失败：signature mismatch");
    await user.click(within(failed).getByRole("button", { name: "重试" }));
    await waitFor(() => {
      expect(screen.getByTestId("update-dialog")).toHaveAttribute("data-state", "checking");
    });
    const status = screen.getByRole("dialog", { name: "软件更新" });
    await user.click(within(status).getByRole("button", { name: "关闭" }));
    expect(screen.queryByRole("dialog", { name: "软件更新" })).toBeNull();
  });
});
