import { type DeviceView, type HistoryEntry, MAX_PASTE_TEXT_CHARS } from "@voltip/shared";
import {
  MOCK_PUBLIC_KEYS,
  MockBackend,
  type MockMirror,
  mockMirrorProfile,
} from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";

// A computer's history and settings on the phone (docs/dictation.md §20.8; user decision
// 2026-10-02: the 记录 tab switches between this phone and each computer; the computer's settings
// are under 设置 › 电脑, read-only).
const NOW = Date.now();
const DESK = MOCK_PUBLIC_KEYS.laptop;

function take(n: number, extra: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`,
    at_ms: NOW - n * 60_000,
    raw_text: `原文 ${n}`,
    text: `第 ${n} 条记录。`,
    refined: true,
    refine_model: "clean-up",
    asr_model: "Qwen/Qwen3-ASR-1.7B",
    duration_ms: 3000,
    asr_ms: 300,
    refine_ms: 200,
    outcome: { kind: "inserted", via: "clipboard" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
    ...extra,
  };
}

const COMPUTER: DeviceView = {
  device: {
    device_id: "b08f44e7-5a91-4e2d-8c3f-071b5c3f071b",
    name: "MacBook Pro",
    platform: "macos",
    public_key: DESK,
    fingerprint: "B0:8F:44:E7 · 5A:91:E2:D8",
    trusted_at: Math.floor(NOW / 1000) - 86_400,
    last_connection: "relay",
    sync: true,
    sync_gen: 0,
  },
  connection: { state: "online", via: "relay" },
};

function mirror(extra: Partial<MockMirror> = {}): MockMirror {
  const history = [
    take(1, { text: "电脑上说的话。" }),
    take(2, { origin: { device: "Pixel 8", kind: "standalone" }, text: "手机上传的副本。" }),
  ];
  return {
    view: {
      desktop: DESK,
      name: "MacBook Pro",
      state: "up_to_date",
      entries: history.length,
      synced_at_ms: NOW,
    },
    history,
    profile: mockMirrorProfile(undefined, {
      dictionary: [
        {
          id: "3b241101-e2bb-4255-8caf-4136c566a962",
          term: "Voltip",
          heard_as: ["沃尔提普"],
          enabled: true,
          source: { kind: "manual" },
          created_at_ms: 1,
          updated_at_ms: 1,
        },
      ],
    }),
    ...extra,
  };
}

function phone(
  mirrors: MockMirror[] = [mirror()],
  history: HistoryEntry[] = [take(5, { text: "这部手机的记录。" })],
) {
  return new MockBackend({ role: "phone", devices: [COMPUTER], mirrors, history });
}

describe("a computer's history on the phone", () => {
  it("the 记录 tab switches to a computer and back, and an entry opened there returns to it", async () => {
    const user = userEvent.setup();
    const backend = phone();
    renderApp({ backend });
    await user.click(await screen.findByTestId("tab-history"));
    const page = screen.getByTestId("phone-history");
    const source = within(page).getByRole("radiogroup", { name: "记录来源" });
    expect(
      within(source)
        .getAllByRole("radio")
        .map((r) => r.textContent),
    ).toEqual(["这部手机", "MacBook Pro"]);
    expect(await within(page).findByText("这部手机的记录。")).toBeInTheDocument();
    expect(within(page).getByTestId("phone-history-stats")).toBeInTheDocument();

    await user.click(within(source).getByRole("radio", { name: "MacBook Pro" }));
    expect(await within(page).findByText("电脑上说的话。")).toBeInTheDocument();
    expect(within(page).queryByText("这部手机的记录。")).toBeNull();
    expect(within(page).queryByTestId("phone-history-stats")).toBeNull();
    expect(within(page).getByTestId("phone-history-mirror-state")).toHaveTextContent(
      "已同步 2 条 · 刚刚",
    );
    // A copy the phone uploaded is named after it.
    expect(within(page).getByText("手机 · Pixel 8")).toHaveAttribute("data-origin", "standalone");

    await user.click(within(page).getByText("电脑上说的话。"));
    const entry = await screen.findByTestId("phone-mirror-entry");
    expect(screen.getByRole("heading", { name: "电脑上的记录", level: 1 })).toBeInTheDocument();
    expect(within(entry).getByTestId("phone-mirror-entry-text")).toHaveTextContent(
      "电脑上说的话。",
    );
    expect(within(entry).queryByRole("button", { name: "收藏" })).toBeNull();
    expect(within(entry).queryByRole("button", { name: "删除" })).toBeNull();
    expect(
      within(entry).getByText("电脑上的记录在手机上只能查看，不能收藏或删除。"),
    ).toBeInTheDocument();
    await user.click(within(entry).getByRole("button", { name: "复制" }));
    await waitFor(async () => {
      expect(await backend.phoneClipboardRead()).toBe("电脑上说的话。");
    });

    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(await screen.findByTestId("phone-history-mirror-state")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "MacBook Pro" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    // The tab bar returns to this phone.
    await user.click(screen.getByTestId("tab-settings"));
    await user.click(screen.getByTestId("tab-history"));
    expect(await screen.findByText("这部手机的记录。")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "这部手机" })).toHaveAttribute("aria-checked", "true");
    backend.destroy();
  });

  it("a shortened entry says so, and one past the copy limit cannot be copied or shared", async () => {
    const user = userEvent.setup();
    const long = take(3, { text: "字".repeat(MAX_PASTE_TEXT_CHARS + 1) });
    const backend = phone([mirror({ history: [long], shortened: [long.id] })]);
    renderApp({ backend, initialScreen: "history" });
    await user.click(await screen.findByRole("radio", { name: "MacBook Pro" }));
    await user.click(await screen.findByTestId("phone-history-row"));
    const entry = await screen.findByTestId("phone-mirror-entry");
    expect(within(entry).getByTestId("phone-mirror-entry-shortened")).toHaveTextContent(
      "这条记录太长",
    );
    expect(within(entry).getByRole("button", { name: "复制" })).toBeDisabled();
    expect(within(entry).getByRole("button", { name: "分享" })).toBeDisabled();
    expect(within(entry).getByTestId("phone-mirror-entry-too-long")).toHaveTextContent(
      "超过 5 万字",
    );
    backend.destroy();
  });

  it("each state of a copy is worded, and a revoked copy is empty", async () => {
    const user = userEvent.setup();
    const backend = phone();
    renderApp({ backend, initialScreen: "history" });
    await user.click(await screen.findByRole("radio", { name: "MacBook Pro" }));
    act(() => {
      backend.simulateMirror({
        view: { desktop: DESK, name: "MacBook Pro", state: "revoked", entries: 0 },
        history: [],
        profile: null,
      });
    });
    expect(await screen.findByText("这台电脑关闭了同步")).toBeInTheDocument();
    expect(await screen.findByText("尚无记录")).toBeInTheDocument();
    for (const [state, text] of [
      ["needs_upgrade", "电脑上的 Voltip 需要升级才能同步"],
      ["limit", "最多同步 5 台电脑，这台电脑没有同步"],
      ["syncing", "正在同步 · 已收到 0 条"],
    ] as const) {
      act(() => {
        backend.simulateMirror({
          view: { desktop: DESK, name: "MacBook Pro", state, entries: 0 },
          history: [],
          profile: null,
        });
      });
      expect(await screen.findByText(text)).toBeInTheDocument();
    }
    backend.destroy();
  });
});

describe("a computer's settings on the phone", () => {
  it("设置 › 电脑 lists the computer and opens its settings, read-only", async () => {
    const user = userEvent.setup();
    const backend = phone();
    renderApp({ backend, initialScreen: "settings" });
    const row = await screen.findByTestId(`settings-computerSettings-${DESK}`);
    expect(row).toHaveTextContent("MacBook Pro 的设置");
    expect(row).toHaveTextContent("已同步 2 条");
    await user.click(row);
    const page = await screen.findByTestId("phone-computer-settings");
    expect(screen.getByRole("heading", { name: "电脑设置", level: 1 })).toBeInTheDocument();
    expect(
      within(page).getByText("MacBook Pro 的设置，在手机上只能查看；在电脑上修改后会同步到手机。"),
    ).toBeInTheDocument();
    expect(within(page).getByRole("region", { name: "外观" })).toHaveTextContent("跟随系统");
    expect(within(page).getByRole("region", { name: "个人词典" })).toHaveTextContent("Voltip");
    expect(within(page).getByRole("region", { name: "个人词典" })).toHaveTextContent(
      "听成：沃尔提普",
    );
    expect(within(page).getByRole("region", { name: "替换规则" })).toHaveTextContent("无");
    expect(within(page).queryByRole("button")).toBeNull();
    backend.destroy();
  });

  it("before any settings arrived the page says so", async () => {
    const user = userEvent.setup();
    const backend = phone([mirror({ profile: null })]);
    renderApp({ backend, initialScreen: "settings" });
    await user.click(await screen.findByTestId(`settings-computerSettings-${DESK}`));
    expect(await screen.findByText("尚未收到这台电脑的设置。")).toBeInTheDocument();
    backend.destroy();
  });
});

describe("the phone's other sync surfaces", () => {
  it("the computer's card says how its copy stands, and forgetting it says the copy goes", async () => {
    const user = userEvent.setup();
    const backend = phone();
    renderApp({ backend, initialScreen: "devices" });
    const card = await screen.findByTestId("device-card");
    expect(within(card).getByTestId("device-sync")).toHaveTextContent("已同步 2 条 · 刚刚");
    await user.click(within(card).getByRole("button", { name: "忘记 MacBook Pro" }));
    expect(await screen.findByText(/这台电脑同步到手机的记录和设置也会删除/)).toBeInTheDocument();
    backend.destroy();
  });

  it("the phone's own entry too large to upload says so", async () => {
    const user = userEvent.setup();
    const own = take(7, { text: "很长的一条。" });
    const backend = phone([mirror()], [own]);
    renderApp({ backend, initialScreen: "history" });
    // The interface folds events only once its first state is in.
    const row = await screen.findByText("很长的一条。");
    act(() => {
      backend.simulateTooLarge([own.id]);
    });
    await user.click(row);
    expect(await screen.findByTestId("phone-entry-too-large")).toHaveTextContent(
      "这条记录太大，没有上传到电脑。",
    );
    backend.destroy();
  });
});
