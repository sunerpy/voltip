import { MAX_PRESET_PROMPT_CHARS, type CustomPreset } from "@voltip/shared";
import {
  MOCK_BUILTIN_PRESET_TEXTS,
  MOCK_ENGINE_BUILTIN,
  MOCK_PRESET_SAMPLES,
  MockBackend,
} from "@voltip/shared/mock";
import { act, render, renderHook, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { BackendProvider } from "../../backend/BackendProvider";
import { I18nProvider } from "../../i18n/I18nProvider";
import { type FeatureConfirm, FeatureShellProvider, useFeatureShell } from "../shell";
import { PresetsSection } from "./PresetsSection";
import { PRESET_TRY_WAIT_MS, usePresetTrial } from "./usePresetTrial";

const WEEKLY: CustomPreset = {
  id: "7e57ab1e-0b0e-4c0d-9e5e-7e57ab1e0b0e",
  name: "Weekly",
  prompt: "整理成周报：按项目分组。",
  created_at_ms: 1,
  updated_at_ms: 1,
};

function renderSection(backend: MockBackend) {
  const notify = vi.fn<(message: string, tone?: "neutral" | "danger") => void>();
  const confirms: FeatureConfirm[] = [];
  render(
    <BackendProvider backend={backend}>
      <I18nProvider locale="zh-CN">
        <FeatureShellProvider shell={{ notify, confirm: (spec) => confirms.push(spec) }}>
          <PresetsSection />
        </FeatureShellProvider>
      </I18nProvider>
    </BackendProvider>,
  );
  return { notify, confirms };
}

describe("PresetsSection (desktop AI 模型 page and phone settings, docs/dictation.md section 21)", () => {
  it("uses a built-in preset and saves refine_preset with the rest of the engine settings", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    renderSection(backend);
    const section = await screen.findByTestId("presets-section");
    const builtin = within(section).getByRole("listbox", { name: "内置预设" });
    expect(within(section).getByTestId("presets-current")).toHaveTextContent("当前：校对");
    expect(within(section).getByTestId("presets-empty")).toBeInTheDocument();
    await user.click(within(builtin).getByRole("option", { name: "提示词优化" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.refine_preset).toBe("prompt");
    });
    expect(backend.peek().settings.engines.refine_enabled).toBe(true);
    backend.destroy();
  });

  it("copies a built-in preset, edits the copy, and deletes it after a confirmation", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const { notify, confirms } = renderSection(backend);
    const section = await screen.findByTestId("presets-section");
    const builtin = within(section).getByRole("listbox", { name: "内置预设" });
    const card = within(builtin).getByRole("option", { name: "提示词优化" });
    await user.click(within(card).getByRole("button", { name: "复制为自定义" }));
    const dialog = await screen.findByRole("dialog", { name: "新建预设" });
    const body = MOCK_BUILTIN_PRESET_TEXTS.find((p) => p.id === "prompt")?.prompt ?? "";
    expect(within(dialog).getByRole("textbox", { name: "名称" })).toHaveValue("提示词优化（副本）");
    expect(within(dialog).getByRole("textbox", { name: "提示词" })).toHaveValue(body);
    await user.click(within(dialog).getByRole("button", { name: /保存/ }));
    await waitFor(() => {
      expect(notify).toHaveBeenCalledWith("已保存预设 · 提示词优化（副本）");
    });
    const custom = within(section).getByRole("listbox", { name: "自定义预设" });
    await user.click(within(custom).getByRole("button", { name: "编辑 提示词优化（副本）" }));
    const editor = await screen.findByRole("dialog", { name: "编辑预设" });
    const name = within(editor).getByRole("textbox", { name: "名称" });
    await user.clear(name);
    await user.type(name, "需求整理");
    await user.click(within(editor).getByRole("button", { name: /保存/ }));
    await waitFor(() => {
      expect(backend.peek().presets.map((p) => p.name)).toEqual(["需求整理"]);
    });
    await user.click(within(section).getByRole("button", { name: "删除 需求整理" }));
    expect(confirms.map((c) => c.title)).toEqual(["删除预设「需求整理」？"]);
    act(() => {
      confirms[0]?.onConfirm();
    });
    await waitFor(() => {
      expect(backend.peek().presets).toEqual([]);
    });
    expect(notify).toHaveBeenLastCalledWith("已删除预设 · 需求整理");
    backend.destroy();
  });

  it("checks the editor like the core, counts the prompt and runs the trial", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ presets: [WEEKLY] });
    renderSection(backend);
    const section = await screen.findByTestId("presets-section");
    await user.click(within(section).getByRole("button", { name: "新建预设" }));
    const dialog = await screen.findByRole("dialog", { name: "新建预设" });
    await user.click(within(dialog).getByRole("button", { name: /保存/ }));
    expect(within(dialog).getByText("请填写名称")).toBeInTheDocument();
    await user.type(within(dialog).getByRole("textbox", { name: "名称" }), "weekly");
    expect(within(dialog).getByText("已有名为「Weekly」的预设")).toBeInTheDocument();
    await user.type(within(dialog).getByRole("textbox", { name: "提示词" }), "  整理成会议纪要  ");
    expect(within(dialog).getByTestId("preset-prompt-count")).toHaveTextContent(
      `7 / ${MAX_PRESET_PROMPT_CHARS}`,
    );
    const trial = within(dialog).getByTestId("preset-trial");
    expect(within(trial).getByRole("textbox", { name: "示例文字" })).toHaveValue(
      MOCK_PRESET_SAMPLES.proofread?.input,
    );
    await user.click(within(trial).getByRole("button", { name: "运行" }));
    const result = await within(trial).findByTestId("preset-trial-result");
    expect(result).toHaveTextContent(`结果 · ${MOCK_ENGINE_BUILTIN.refine_model} · 300 ms`);
    await user.clear(within(dialog).getByRole("textbox", { name: "名称" }));
    await user.type(within(dialog).getByRole("textbox", { name: "名称" }), "会议纪要");
    await user.click(within(dialog).getByRole("button", { name: /保存/ }));
    await waitFor(() => {
      expect(backend.peek().presets.map((p) => p.name)).toEqual(["Weekly", "会议纪要"]);
    });
    backend.destroy();
  });

  it("shows a refusal from the core in the editor and a failed copy as a danger notice", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const { notify } = renderSection(backend);
    const section = await screen.findByTestId("presets-section");
    vi.spyOn(backend, "presetsBuiltin").mockRejectedValueOnce(new Error("presets: 读取失败"));
    const card = within(within(section).getByRole("listbox", { name: "内置预设" })).getByRole(
      "option",
      { name: "校对" },
    );
    await user.click(within(card).getByRole("button", { name: "复制为自定义" }));
    await waitFor(() => {
      // The machine prefix stays out of the interface (`coreMessageText`).
      expect(notify).toHaveBeenCalledWith("读取失败", "danger");
    });
    await user.click(within(section).getByRole("button", { name: "新建预设" }));
    const dialog = await screen.findByRole("dialog", { name: "新建预设" });
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("presets: 名称重复"));
    await user.type(within(dialog).getByRole("textbox", { name: "名称" }), "周报");
    await user.type(within(dialog).getByRole("textbox", { name: "提示词" }), "整理成周报");
    await user.click(within(dialog).getByRole("button", { name: /保存/ }));
    expect(await within(dialog).findByTestId("preset-editor-error")).toHaveTextContent("名称重复");
    backend.destroy();
  });

  it("without an AI polish service the trial is unavailable and says why", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ builtIn: {} });
    renderSection(backend);
    const section = await screen.findByTestId("presets-section");
    await user.click(within(section).getByRole("button", { name: "新建预设" }));
    const trial = within(await screen.findByRole("dialog", { name: "新建预设" })).getByTestId(
      "preset-trial",
    );
    expect(trial).toHaveTextContent("尚未配置 AI 润色服务，无法试运行。");
    expect(within(trial).getByRole("button", { name: "运行" })).toBeDisabled();
    backend.destroy();
  });
});

describe("usePresetTrial", () => {
  function wrapper(backend: MockBackend) {
    return ({ children }: { children: ReactNode }) => (
      <BackendProvider backend={backend}>
        <I18nProvider locale="zh-CN">{children}</I18nProvider>
      </BackendProvider>
    );
  }

  it("gives up after its wait and reports a refused request", async () => {
    vi.useFakeTimers();
    const backend = new MockBackend();
    try {
      vi.spyOn(backend, "invoke").mockReturnValueOnce(new Promise(() => undefined));
      const { result } = renderHook(() => usePresetTrial(), { wrapper: wrapper(backend) });
      act(() => {
        result.current.run({ prompt: "整理" }, "你好");
      });
      expect(result.current.pending).toBe(true);
      act(() => {
        vi.advanceTimersByTime(PRESET_TRY_WAIT_MS);
      });
      expect(result.current.outcome).toEqual({ status: "failed", reason: "等待超时，请稍后重试" });
      act(() => {
        result.current.reset();
      });
      expect(result.current.outcome).toBeUndefined();
      vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("presets: 未配置"));
      act(() => {
        result.current.run({ preset: "proofread" }, "你好");
      });
      await act(async () => {
        await Promise.resolve();
      });
      expect(result.current.outcome).toEqual({ status: "failed", reason: "未配置" });
    } finally {
      vi.useRealTimers();
      backend.destroy();
    }
  });
});

describe("useFeatureShell", () => {
  it("is refused outside a provider", () => {
    expect(() => renderHook(() => useFeatureShell())).toThrow(/FeatureShellProvider/);
  });
});
