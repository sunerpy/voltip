import { defaultEngineSettings } from "@voltip/shared";
import {
  MOCK_ASR_MS,
  MOCK_ENGINE_BUILTIN,
  MOCK_METER_INTERVAL_MS,
  MOCK_MIC_READY_MS,
  MOCK_REFINE_MS,
  MockBackend,
  type MockBackendOptions,
  sampleUsage,
} from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";

/** A build with the built-in recognition and clean-up, as a release build carries them. */
const BUILT_IN: MockBackendOptions["builtIn"] = {
  asr: { model: MOCK_ENGINE_BUILTIN.asr_model, key: true },
  llm: { model: MOCK_ENGINE_BUILTIN.refine_model, key: true },
};

/** The full card's container query: the class that shows it on a tall enough sidebar. */
const FULL_QUERY = "[@container_sidebar_(min-height:calc(38rem_+_160px))]:flex";

function withUsage(state: Parameters<typeof sampleUsage>[0], extra: MockBackendOptions = {}) {
  return renderApp({ mock: { builtIn: BUILT_IN, usage: sampleUsage(state), ...extra } });
}

describe("usage of the built-in service (docs/dictation.md section 27)", () => {
  describe("the sidebar card", () => {
    it("shows both services against their limits, one line on a low window, and opens 用量", async () => {
      const { backend } = withUsage("ok");
      const card = await screen.findByTestId("sidebar-usage-card");
      expect(card).toHaveAttribute("data-mode", "builtin");
      expect(card).toHaveAttribute("data-tone", "ok");
      expect(card).toHaveClass("hidden", FULL_QUERY);
      expect(within(card).getByTestId("sidebar-usage-asr")).toHaveTextContent("识别23 / 60 分钟");
      expect(within(card).getByTestId("sidebar-usage-llm")).toHaveTextContent("润色41 / 100 次");
      expect(
        within(card)
          .getAllByRole("meter")
          .map((m) => m.getAttribute("aria-valuenow")),
      ).toEqual(["38", "41"]);
      expect(card).toHaveTextContent(/\d\d:\d\d 重置/);
      // The one-line form names the tighter service.
      const compact = screen.getByTestId("sidebar-usage-compact");
      expect(compact).toHaveTextContent("今日用量 · 润色 41 / 100 次");
      const spy = vi.spyOn(backend, "invoke");
      await userEvent.click(card);
      expect(await screen.findByTestId("page-usage")).toBeInTheDocument();
      expect(screen.getByTestId("sidebar-usage-card")).toHaveAttribute("aria-current", "page");
      // The page asks the service again as it opens.
      expect(spy).toHaveBeenCalledWith("usage_refresh");
    });

    it("regression: the English heading stays on one line beside the reset time", async () => {
      // "Today's usage" wrapped onto two lines next to "Resets 00:00" in the sidebar's width.
      renderApp({ mock: { builtIn: BUILT_IN, usage: sampleUsage("ok") }, systemLanguage: "en-US" });
      const card = await screen.findByTestId("sidebar-usage-card");
      const title = within(card).getByTestId("sidebar-usage-title");
      expect(title).toHaveTextContent(/^Usage$/);
      expect(title).toHaveClass("shrink-0", "whitespace-nowrap");
      expect(card).toHaveTextContent(/Resets \d\d:\d\d/);
    });

    it("turns amber from 80 % and red at the limit, the one line saying what is left", async () => {
      withUsage("near");
      const card = await screen.findByTestId("sidebar-usage-card");
      expect(card).toHaveAttribute("data-tone", "near");
      expect(within(card).getByTestId("sidebar-usage-asr")).toHaveTextContent("剩余 8 分钟");
      // Recognition is the tighter one (52 of 60 minutes, 86 of 100 clean-ups).
      expect(screen.getByTestId("sidebar-usage-compact")).toHaveTextContent(
        "今日用量 · 识别剩余 8 分钟",
      );
    });

    it("says 暂不限量 in the counting period and 无法获取 when the service cannot be asked", async () => {
      const { unmount } = withUsage("count");
      const card = await screen.findByTestId("sidebar-usage-card");
      expect(card).toHaveAttribute("data-mode", "unlimited");
      expect(card).toHaveTextContent("暂不限量");
      expect(card).toHaveTextContent("统计期间不设上限");
      expect(within(card).queryAllByRole("meter")).toHaveLength(0);
      unmount();
      withUsage("unknown");
      const unknown = await screen.findByTestId("sidebar-usage-card");
      expect(unknown).toHaveAttribute("data-mode", "unknown");
      expect(unknown).toHaveTextContent("用量暂时无法获取");
      expect(screen.getByTestId("sidebar-usage-compact")).toHaveTextContent("今日用量 · 无法获取");
    });

    it("counts this device's own use while neither service uses the built-in one", async () => {
      withUsage("ok", {
        settings: {
          engines: { ...defaultEngineSettings(), asr_provider: "local", llm_provider: "groq" },
        },
      });
      const card = await screen.findByTestId("sidebar-usage-card");
      expect(card).toHaveAttribute("data-mode", "own");
      expect(card).toHaveTextContent("本机统计");
      expect(within(card).getByTestId("sidebar-usage-asr")).toHaveTextContent("7 分钟 · 本机");
      expect(within(card).getByTestId("sidebar-usage-llm")).toHaveTextContent("0 次");
      expect(card).toHaveTextContent("未使用内置服务，不限量");
    });

    it("is two rings with the numbers in its name in the icon rail, and absent without the built-in service", async () => {
      const { unmount } = withUsage("near");
      await userEvent.click(await screen.findByTestId("sidebar-collapse"));
      const rail = await screen.findByTestId("sidebar-usage-rail");
      expect(rail).toHaveAccessibleName(
        /^今日用量：识别 剩余 8 分钟，润色 剩余 14 次 · \d\d:\d\d 重置$/,
      );
      expect(rail.querySelectorAll("circle")).toHaveLength(4);
      unmount();
      // The rail stays (the layout is kept); a build without the built-in service has no card.
      renderApp({ mock: { builtIn: {} } });
      await screen.findByTestId("sidebar-footer");
      expect(screen.queryByTestId("sidebar-usage-rail")).toBeNull();
      await userEvent.click(screen.getByTestId("sidebar-collapse"));
      await screen.findByTestId("sidebar-status-card");
      expect(screen.queryByTestId("sidebar-usage-card")).toBeNull();
      expect(screen.queryByTestId("sidebar-usage-compact")).toBeNull();
    });
  });

  describe("the 用量 page", () => {
    it("shows each service's day, the week, today's breakdown and the other providers", async () => {
      withUsage("near");
      await userEvent.click(await screen.findByTestId("sidebar-usage-card"));
      const asr = await screen.findByTestId("usage-card-asr");
      expect(within(asr).getByTestId("usage-numbers-asr")).toHaveTextContent(
        "52/ 60 分钟剩余 8 分钟",
      );
      expect(within(asr).getByTestId("usage-reset-asr")).toHaveTextContent(/重置 · (还有|不到)/);
      expect(within(asr).getAllByTestId("usage-week-bar")).toHaveLength(7);
      const llm = screen.getByTestId("usage-card-llm");
      expect(within(llm).getByTestId("usage-numbers-llm")).toHaveTextContent(
        "86/ 100 次剩余 14 次",
      );
      const detail = screen.getByTestId("usage-detail");
      expect(within(detail).getByTestId("usage-detail-asr")).toHaveTextContent(
        "这台电脑直接听写47 分钟",
      );
      expect(within(detail).getByTestId("usage-detail-asr")).toHaveTextContent(
        "Pixel 8手机发来的录音5 分钟",
      );
      expect(within(detail).getByTestId("usage-detail-llm")).toHaveTextContent("语音编辑");
      const others = screen.getAllByTestId("usage-other");
      expect(others.map((row) => row.textContent)).toEqual([
        "本机qwen3-asr-0.6b识别 2 小时 10 分不限量 · 在这台电脑上运行",
        expect.stringMatching(
          /^阿里云百炼paraformer-realtime-v2识别 38 分钟剩余额度请在百炼控制台查看/,
        ),
        "Groqqwen/qwen3.8-27b润色 312 次剩余额度请在服务商处查看",
      ]);
    });

    it("offers 重试 while the numbers cannot be fetched, and says when a service uses another provider", async () => {
      const { backend } = renderApp({
        path: "/usage",
        mock: {
          builtIn: BUILT_IN,
          usage: sampleUsage("unknown"),
          settings: { engines: { ...defaultEngineSettings(), llm_provider: "groq" } },
        },
      });
      const asr = await screen.findByTestId("usage-card-asr");
      expect(asr).toHaveTextContent("用量暂时无法获取");
      const spy = vi.spyOn(backend, "invoke");
      await userEvent.click(within(asr).getByTestId("usage-retry-asr"));
      expect(spy).toHaveBeenCalledWith("usage_refresh");
      expect(screen.getByTestId("usage-not-in-use-llm")).toHaveTextContent(
        "润色现在使用的是Groq，不计入内置服务的额度",
      );
      expect(screen.queryByTestId("usage-not-in-use-asr")).toBeNull();
    });

    it("is in the command palette of a build with the built-in service", async () => {
      withUsage("ok");
      await screen.findByTestId("sidebar-usage-card");
      await userEvent.keyboard("{Control>}k{/Control}");
      await userEvent.click(await screen.findByRole("option", { name: /打开用量/ }));
      expect(await screen.findByTestId("page-usage")).toBeInTheDocument();
    });
  });

  describe("the home page", () => {
    it("shows the usage under each model and a notice from 80 %, closed for the day", async () => {
      const { backend } = withUsage("near");
      expect(await screen.findByTestId("home-engine-usage")).toHaveTextContent(
        "内置服务 · 今日剩余 8 分钟",
      );
      expect(screen.getByTestId("home-refine-usage")).toHaveTextContent("· 今日剩余 14 次");
      expect(screen.getByTestId("usage-notice-asr_near")).toHaveTextContent(
        /^明天 00:00 重置。用完后可以改用本机模型|重置。用完后可以改用本机模型/,
      );
      expect(screen.getByTestId("usage-notice-refine_near")).toBeInTheDocument();
      const banner = screen.getByTestId("usage-notice-asr_near").closest("[role]");
      if (!(banner instanceof HTMLElement)) throw new Error("banner");
      expect(banner).toHaveTextContent("内置语音识别今日剩余 8 分钟");
      const spy = vi.spyOn(backend, "invoke");
      await userEvent.click(within(banner).getByRole("button", { name: "关闭" }));
      expect(spy).toHaveBeenCalledWith("usage_notice_dismiss", { notice: "asr_near" });
      await waitFor(() => {
        expect(screen.queryByTestId("usage-notice-asr_near")).toBeNull();
      });
      expect(screen.getByTestId("usage-notice-refine_near")).toBeInTheDocument();
    });

    it("says on the 听写 card when the built-in recognition is used up, with the ways on", async () => {
      withUsage("out");
      const hero = await screen.findByTestId("home-hero");
      expect(within(hero).getByTestId("home-readiness")).toHaveTextContent(
        "内置语音识别今日额度已用完",
      );
      expect(within(hero).getByTestId("home-phase")).toHaveTextContent(
        /重置；现在可以改用本机模型或其他服务商/,
      );
      expect(within(hero).getByTestId("home-hint")).toHaveTextContent("快捷键仍然可用");
      expect(within(hero).getByTestId("home-mic-state")).toHaveTextContent("内置识别不可用");
      // Still a take: the recording is kept to recognise again.
      expect(within(hero).getByTestId("home-start")).toBeEnabled();
      expect(screen.getByTestId("usage-notice-asr_out")).toHaveTextContent(/录音保留在本机/);
      expect(screen.getByTestId("usage-notice-refine_out")).toBeInTheDocument();
      // Nothing on-device yet: 下载本机模型 opens the on-device models' drawer.
      await userEvent.click(within(hero).getByTestId("home-use-local"));
      expect(await screen.findByTestId("provider-asr-local-detail")).toBeInTheDocument();
    });

    it("switches recognition to an installed on-device model", async () => {
      const { backend } = renderApp({
        mock: {
          builtIn: BUILT_IN,
          usage: sampleUsage("out"),
          models: {
            "qwen3-asr-0.6b": {
              kind: "installed",
              path: "~/.local/share/voltip/models/qwen3-asr-0.6b",
              installed_at: 1_758_600_000,
            },
          },
        },
      });
      const button = await screen.findByTestId("home-use-local");
      expect(button).toHaveTextContent("改用本机模型");
      await userEvent.click(button);
      await waitFor(() => {
        expect(backend.peek().settings.engines.asr_provider).toBe("local");
      });
      expect(backend.peek().settings.engines.local_model).toBe("qwen3-asr-0.6b");
      expect(screen.queryByTestId("home-usage-out")).toBeNull();
    });

    it("keeps the card ready while a fallback model stands in, and the notice says which", async () => {
      // The build offers a second built-in model; the fallback list moves on to it.
      const backend = new MockBackend({
        now: () => 1_758_700_000_000,
        builtIn: {
          ...BUILT_IN,
          asr: {
            model: MOCK_ENGINE_BUILTIN.asr_model,
            models: [MOCK_ENGINE_BUILTIN.asr_model, "qwen3-asr-flash"],
            key: true,
          },
        },
        usage: sampleUsage("out"),
        settings: {
          engines: {
            ...defaultEngineSettings(),
            asr_fallback: {
              enabled: true,
              models: [{ provider: "builtin", model: "qwen3-asr-flash" }],
            },
          },
        },
      });
      renderApp({ backend });
      const notice = await screen.findByTestId("usage-notice-asr_out");
      expect(backend.peek().engines.asr_fallback?.in_use).toBe(true);
      expect(notice).toHaveTextContent(/在此之前由候补模型 qwen3-asr-flash 识别/);
      expect(screen.queryByTestId("home-usage-out")).toBeNull();
      expect(screen.queryByTestId("usage-use-local")).toBeNull();
      expect(screen.getByTestId("home-readiness")).toHaveTextContent("可以开始听写");
    });
  });

  describe("the pill", () => {
    it("says the built-in recognition's day is used up and the recording kept, and why a take was not polished", async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      try {
        const backend = new MockBackend({
          now: () => Date.now(),
          builtIn: BUILT_IN,
          usage: sampleUsage("out"),
        });
        renderApp({ path: "/overlay?state=live", backend });
        await screen.findByTestId("overlay-window");
        act(() => {
          backend.simulateDictationFailed("asr: quota", undefined, "quota");
        });
        const failed = await screen.findByRole("status");
        expect(failed).toHaveTextContent("今日内置识别额度已用完");
        backend.simulateRefineFailure("quota");
        await act(async () => {
          await backend.invoke("dictation_start");
        });
        await act(async () => {
          await vi.advanceTimersByTimeAsync(MOCK_MIC_READY_MS + MOCK_METER_INTERVAL_MS * 2);
        });
        await act(async () => {
          await backend.invoke("dictation_stop");
        });
        await act(async () => {
          await vi.advanceTimersByTimeAsync(MOCK_ASR_MS + MOCK_REFINE_MS);
        });
        expect(screen.getByTestId("pill-note")).toHaveTextContent(
          "未润色 · 今日 AI 润色额度已用完",
        );
      } finally {
        vi.useRealTimers();
      }
    });
  });

  describe("the built-in service's drawer", () => {
    it("shows today's use, how it is counted, the week and today's sources", async () => {
      const { backend } = renderApp({
        path: "/speech",
        mock: { builtIn: BUILT_IN, usage: sampleUsage("near") },
      });
      const card = await screen.findByTestId("provider-asr-builtin");
      expect(within(card).getByTestId("provider-asr-usage")).toHaveTextContent("今日剩余 8 分钟");
      await userEvent.click(within(card).getByRole("button", { name: "内置服务" }));
      const drawer = await screen.findByTestId("builtin-usage-asr");
      expect(drawer).toHaveTextContent("内置语音识别");
      expect(drawer).toHaveTextContent("剩余 8 分钟");
      expect(drawer).toHaveTextContent("已用 52 / 60 分钟");
      expect(drawer).toHaveTextContent("已配对手机发到这台电脑识别的录音也计入这里");
      expect(within(drawer).getAllByTestId("usage-share")).toHaveLength(2);
      const spy = vi.spyOn(backend, "guideOpen");
      await userEvent.click(within(drawer).getByTestId("usage-rules"));
      expect(spy).toHaveBeenCalledWith("usage", "zh-CN");
    });
  });
});
