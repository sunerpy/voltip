import { type Backend, type ServiceUsage, defaultEngineSettings } from "@voltip/shared";
import { MockBackend, sampleUsage } from "@voltip/shared/mock";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { BackendProvider, useUiState } from "../../backend/BackendProvider";
import { Sidebar } from "../../components/Sidebar";
import { I18nProvider } from "../../i18n/I18nProvider";
import { ProviderCards } from "../engines/ProviderCard";
import { FeatureShellProvider } from "../shell";
import {
  BuiltinUsage,
  UsageBar,
  UsageInline,
  UsageMeter,
  UsageShares,
  UsageWeekChart,
  useServiceUsage,
} from "./Usage";

function shell(backend: Backend, ui: ReactNode, locale: "zh-CN" | "en" = "zh-CN") {
  return render(
    <BackendProvider backend={backend}>
      <I18nProvider locale={locale}>
        <FeatureShellProvider shell={{ notify: () => undefined, confirm: () => undefined }}>
          {ui}
        </FeatureShellProvider>
      </I18nProvider>
    </BackendProvider>,
  );
}

const usage = (over: Partial<ServiceUsage>): ServiceUsage => ({
  kind: "asr",
  tone: "ok",
  used: 23,
  limit: 60,
  left: 37,
  leftUnderOne: false,
  share: 23 / 60,
  resetAtMs: Date.now() + 3_600_000,
  ...over,
});

/** `kind`'s usage from the backend's state, through the meter. */
function Meter({ kind, onRetry }: { kind: "asr" | "llm"; onRetry?: () => void }) {
  const u = useServiceUsage(kind);
  const { usage: view } = useUiState();
  if (!view.builtin) return null;
  return <UsageMeter usage={u} {...(onRetry === undefined ? {} : { onRetry })} />;
}

describe("UsageBar", () => {
  it("is a meter filled to its share, a dot for a little use, empty while unknown", () => {
    render(
      <I18nProvider locale="zh-CN">
        <UsageBar share={0.384} tone="ok" label="识别" />
        <UsageBar share={0.002} tone="near" size={8} width={40} label="润色" />
        <UsageBar share={0} tone="unknown" label="未知" />
      </I18nProvider>,
    );
    const [ok, near, unknown] = screen.getAllByRole("meter");
    expect(ok).toHaveAttribute("aria-valuenow", "38");
    expect(ok).toHaveAccessibleName("识别");
    expect(ok?.firstElementChild).toHaveClass("bg-accent");
    expect(near).toHaveStyle({ width: "40px", height: "8px" });
    const fill = near?.firstElementChild;
    if (!(fill instanceof HTMLElement)) throw new Error("no fill");
    expect(fill).toHaveClass("bg-warning-fill");
    expect(fill.style.width).toBe("max(0%, 8px)");
    expect(unknown?.firstElementChild).toBeNull();
  });
});

describe("UsageInline", () => {
  it("says each state in one line, with a bar while under the limit", () => {
    const backend = new MockBackend();
    shell(
      backend,
      <>
        <UsageInline usage={usage({})} prefix="内置服务 · " data-testid="ok" />
        <UsageInline
          usage={usage({ tone: "near", used: 52, left: 8, share: 52 / 60 })}
          data-testid="near"
        />
        <UsageInline
          usage={usage({ tone: "out", used: 60, left: 0, share: 1 })}
          data-testid="out"
        />
        <UsageInline usage={usage({ tone: "unlimited" })} barWidth={0} data-testid="count" />
      </>,
    );
    expect(screen.getByTestId("ok")).toHaveTextContent("内置服务 · 今日 23 / 60 分钟");
    expect(within(screen.getByTestId("ok")).getByRole("meter")).toHaveStyle({ width: "56px" });
    expect(screen.getByTestId("near")).toHaveTextContent("今日剩余 8 分钟");
    expect(within(screen.getByTestId("near")).getByRole("meter")).toBeInTheDocument();
    expect(screen.getByTestId("out")).toHaveTextContent(/^今日额度已用完 · .*重置$/);
    expect(within(screen.getByTestId("out")).queryByRole("meter")).toBeNull();
    expect(screen.getByTestId("count")).toHaveTextContent("今日已用 23 分钟 · 暂不限量");
  });
});

describe("UsageMeter", () => {
  it("shows what is left, the numbers and the reset; 无法获取 offers 重试", async () => {
    const onRetry = vi.fn();
    const backend = new MockBackend();
    shell(
      backend,
      <>
        <UsageMeter usage={usage({})} data-testid="ok" />
        <UsageMeter
          usage={usage({ kind: "llm", tone: "out", used: 100, limit: 100, left: 0, share: 1 })}
          data-testid="out"
        />
        <UsageMeter usage={usage({ tone: "unlimited" })} data-testid="count" />
        <UsageMeter usage={usage({ tone: "unknown" })} onRetry={onRetry} data-testid="unknown" />
        <UsageMeter usage={usage({ tone: "pending" })} data-testid="pending" />
      </>,
    );
    const ok = screen.getByTestId("ok");
    expect(within(ok).getByTestId("usage-meter-status")).toHaveTextContent("剩余 37 分钟");
    expect(ok).toHaveTextContent("已用 23 / 60 分钟");
    expect(ok).toHaveTextContent(/重置 · 还有/);
    const out = screen.getByTestId("out");
    expect(out).toHaveTextContent("内置 AI 润色");
    expect(within(out).getByTestId("usage-meter-status")).toHaveTextContent("今日已用完");
    expect(within(out).getByRole("meter")).toHaveAttribute("aria-valuenow", "100");
    expect(screen.getByTestId("count")).toHaveTextContent("今日已用 23 分钟");
    expect(screen.getByTestId("count")).toHaveTextContent("统计期间不设上限");
    expect(within(screen.getByTestId("count")).queryByRole("meter")).toBeNull();
    expect(screen.getByTestId("unknown")).toHaveTextContent("用量暂时无法获取");
    await userEvent.click(within(screen.getByTestId("unknown")).getByTestId("usage-retry"));
    expect(onRetry).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId("pending")).toHaveTextContent("正在获取用量…");
    expect(within(screen.getByTestId("pending")).queryByTestId("usage-retry")).toBeNull();
  });

  it("reads the backend's day for a service", async () => {
    const backend = new MockBackend({ usage: sampleUsage("near") });
    shell(backend, <Meter kind="llm" />);
    expect(await screen.findByTestId("usage-meter-status")).toHaveTextContent("剩余 14 次");
  });
});

describe("UsageWeekChart", () => {
  it("charts seven days against the limit, today in the accent and a full day in red", async () => {
    const backend = new MockBackend({ usage: sampleUsage("ok") });
    shell(
      backend,
      <>
        <UsageWeekChart kind="asr" limit={60} />
        <UsageWeekChart kind="llm" />
      </>,
    );
    const asr = screen.getByTestId("usage-week-asr");
    const bars = await within(asr).findAllByTestId("usage-week-bar");
    expect(bars).toHaveLength(7);
    expect(bars[3]).toHaveClass("bg-danger");
    expect(bars[6]).toHaveClass("bg-accent");
    expect(bars[0]).toHaveClass("bg-accent/35");
    expect(asr).toHaveTextContent("60");
    expect(asr.getAttribute("aria-label")).toMatch(/^近 7 天：周.* \d+ 分钟.*今天 23 分钟$/);
    // No limit, no dashed line.
    const llm = screen.getByTestId("usage-week-llm");
    expect(within(llm).getAllByTestId("usage-week-bar")).toHaveLength(7);
    expect(llm.querySelector(".border-dashed")).toBeNull();
  });
});

describe("UsageShares", () => {
  it("lists rows with their share, a phone's name as the user's text", () => {
    render(
      <UsageShares
        rows={[
          { key: "this_device", name: "这台电脑", note: "直接听写", value: "18 分钟", share: 0.78 },
          { key: "phone:Pixel 8", name: "Pixel 8", value: "5 分钟", share: 0.22, soft: true },
        ]}
      />,
    );
    const rows = screen.getAllByTestId("usage-share");
    expect(rows[0]).toHaveTextContent("这台电脑直接听写18 分钟");
    expect(rows[1]?.querySelector("[data-user-text]")).toHaveTextContent("Pixel 8");
    expect(rows[1]?.querySelector(".bg-accent\\/35")).toHaveStyle({ width: "22%" });
  });
});

describe("BuiltinUsage", () => {
  it("is nothing in a build without the built-in service", () => {
    const backend = new MockBackend({ builtIn: {} });
    const { container } = shell(backend, <BuiltinUsage kind="asr" />);
    expect(container).toBeEmptyDOMElement();
  });

  it("shows the clean-up's day by purpose, and nothing used yet as such", async () => {
    const backend = new MockBackend({ usage: sampleUsage("ok") });
    const { unmount } = shell(backend, <BuiltinUsage kind="llm" />);
    const section = await screen.findByTestId("builtin-usage-llm");
    expect(section).toHaveTextContent("今日用途");
    expect(section).toHaveTextContent("共 41 次");
    expect(section).toHaveTextContent("每次 AI 润色和语音编辑计 1 次");
    expect(
      within(section)
        .getAllByTestId("usage-share")
        .map((r) => r.textContent),
    ).toEqual(["AI 润色听写后整理文字36 次", "语音编辑用语音修改选中的文字5 次"]);
    unmount();
    const empty = new MockBackend({
      usage: { ...sampleUsage("unknown"), asr_origins: [], refine_purposes: [] },
    });
    const emptySpy = vi.spyOn(empty, "invoke");
    shell(empty, <BuiltinUsage kind="asr" />, "en");
    const asr = await screen.findByTestId("builtin-usage-asr");
    expect(asr).toHaveTextContent("Not used today");
    expect(asr).toHaveTextContent("Today's sources");
    await userEvent.click(within(asr).getByTestId("usage-retry"));
    expect(emptySpy).toHaveBeenCalledWith("usage_refresh");
  });
});

describe("the provider cards", () => {
  it("show the built-in service's day on its card once it reported, and in its drawer", async () => {
    const backend = new MockBackend({ usage: sampleUsage("near") });
    function Cards() {
      const state = useUiState();
      return (
        <ProviderCards
          kind="asr"
          providers={state.engines.providers.filter((p) => p.asr !== undefined)}
          label="服务商"
        />
      );
    }
    shell(backend, <Cards />);
    const card = await screen.findByTestId("provider-asr-builtin");
    expect(within(card).getByTestId("provider-asr-usage")).toHaveTextContent("今日剩余 8 分钟");
    await userEvent.click(within(card).getByRole("button", { name: "内置服务" }));
    expect(await screen.findByTestId("builtin-usage-asr")).toBeInTheDocument();
  });

  it("show nothing on the card before the first report, and open a drawer at first when asked", async () => {
    const backend = new MockBackend({
      settings: { engines: { ...defaultEngineSettings() } },
    });
    function Cards() {
      const state = useUiState();
      return (
        <ProviderCards
          kind="asr"
          providers={state.engines.providers.filter((p) => p.asr !== undefined)}
          label="服务商"
          initialOpen="builtin"
        />
      );
    }
    shell(backend, <Cards />);
    const card = await screen.findByTestId("provider-asr-builtin");
    expect(within(card).queryByTestId("provider-asr-usage")).toBeNull();
    expect(await screen.findByTestId("provider-asr-builtin-detail")).toBeInTheDocument();
  });
});

describe("Sidebar", () => {
  it("is a size container and carries the usage card in both layouts", () => {
    const { rerender } = render(
      <I18nProvider locale="zh-CN">
        <Sidebar
          groups={[]}
          activeId="home"
          onNavigate={() => undefined}
          usage={<span>用量卡片</span>}
        />
      </I18nProvider>,
    );
    expect(screen.getByRole("navigation")).toHaveClass(
      "[container-type:size]",
      "[container-name:sidebar]",
    );
    expect(screen.getByTestId("sidebar-usage")).toHaveTextContent("用量卡片");
    rerender(
      <I18nProvider locale="zh-CN">
        <Sidebar
          groups={[]}
          activeId="home"
          onNavigate={() => undefined}
          collapsed
          usage={<span>环</span>}
        />
      </I18nProvider>,
    );
    expect(screen.getByTestId("sidebar-usage")).toHaveTextContent("环");
  });
});
