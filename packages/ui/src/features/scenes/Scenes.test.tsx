import {
  type AppRef,
  type HistoryEntry,
  MAX_SCENE_PROMPT_CHARS,
  type Scene,
  defaultEngineSettings,
} from "@voltip/shared";
import { MockBackend, desktopIdentity } from "@voltip/shared/mock";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { BackendProvider } from "../../backend/BackendProvider";
import { I18nProvider } from "../../i18n/I18nProvider";
import { type FeatureConfirm, FeatureShellProvider } from "../shell";
import { SceneCards } from "./SceneCards";
import { SceneEditor } from "./SceneEditor";

/** A desktop whose list holds only the user's scenes (no built-in ones: platform `other`). */
const NO_BUILTIN = { identity: { ...desktopIdentity(), platform: "other" as const } };
const NOW = 1_758_700_000_000;

function scene(id: string, name: string, extra: Partial<Scene> = {}): Scene {
  return {
    id,
    name,
    enabled: true,
    match: { apps: ["app"], title_contains: [] },
    overrides: {},
    created_at_ms: NOW,
    updated_at_ms: NOW,
    ...extra,
  };
}

const CHAT = scene("00000000-0000-4000-a000-000000000001", "聊天", {
  match: { apps: ["slack", "wechat"], title_contains: [] },
  overrides: { refine_enabled: false, refine_preset: "punctuation", prompt: "口语化" },
});
const GITHUB = scene("00000000-0000-4000-a000-000000000002", "GitHub", {
  enabled: false,
  match: { apps: ["chrome"], title_contains: ["GitHub", "Pull request"] },
});

function take(n: number, app: AppRef): HistoryEntry {
  return {
    id: `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`,
    at_ms: NOW - n * 60_000,
    raw_text: "好的",
    text: "好的。",
    refined: false,
    asr_model: "m",
    duration_ms: 1200,
    asr_ms: 300,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
    app,
  };
}

/** The cards and the editor the way the desktop pane and the phone page put them together. */
function Scenes({ matchApps, outputModes }: { matchApps: boolean; outputModes: boolean }) {
  const [editing, setEditing] = useState<{ scene?: Scene } | undefined>(undefined);
  return (
    <>
      <button
        type="button"
        onClick={() => {
          setEditing({});
        }}>
        new scene
      </button>
      <SceneCards
        matchApps={matchApps}
        onEdit={(s) => {
          setEditing({ scene: s });
        }}
      />
      {editing !== undefined && (
        <SceneEditor
          key={editing.scene?.id ?? "new"}
          scene={editing.scene}
          matchApps={matchApps}
          outputModes={outputModes}
          onClose={() => {
            setEditing(undefined);
          }}
        />
      )}
    </>
  );
}

function renderScenes(backend: MockBackend, matchApps = true, outputModes = matchApps) {
  const notify = vi.fn<(message: string, tone?: "neutral" | "danger") => void>();
  const confirms: FeatureConfirm[] = [];
  render(
    <BackendProvider backend={backend}>
      <I18nProvider locale="zh-CN">
        <FeatureShellProvider shell={{ notify, confirm: (spec) => confirms.push(spec) }}>
          <Scenes matchApps={matchApps} outputModes={outputModes} />
        </FeatureShellProvider>
      </I18nProvider>
    </BackendProvider>,
  );
  return { notify, confirms };
}

function cards(): HTMLElement[] {
  return screen.getAllByRole("article");
}

describe("the scene cards and editor (desktop 场景 settings and the phone, docs/dictation.md section 18)", () => {
  it("on the desktop a card shows its order, switch and matches; the switch, the order buttons and delete run the scene commands", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ ...NO_BUILTIN, scenes: [CHAT, GITHUB] });
    const { notify, confirms } = renderScenes(backend);
    await waitFor(() => {
      expect(cards().map((c) => c.getAttribute("aria-label"))).toEqual(["聊天", "GitHub"]);
    });
    const [chat, github] = cards() as [HTMLElement, HTMLElement];
    expect(within(chat).getByTestId("scene-match")).toHaveTextContent("slack");
    expect(within(chat).getByTestId("scene-match")).toHaveTextContent("任何窗口");
    expect(within(github).getByTestId("scene-match")).toHaveTextContent(
      "窗口标题含 GitHub · Pull request",
    );
    expect(within(chat).getByTestId("scene-summary")).toHaveTextContent(
      "AI 润色 关 · AI 预设：只加标点 · 有补充要求",
    );
    expect(within(github).getByTestId("scene-summary")).toHaveTextContent("全部跟随全局设置");

    await user.click(within(github).getByRole("switch", { name: "启用 GitHub" }));
    await waitFor(() => {
      expect(backend.peek().scenes[1]?.enabled).toBe(true);
    });
    await user.click(
      within(cards()[1] as HTMLElement).getByRole("button", { name: "上移 GitHub" }),
    );
    await waitFor(() => {
      expect(backend.peek().scenes.map((s) => s.name)).toEqual(["GitHub", "聊天"]);
    });
    await user.click(within(cards()[1] as HTMLElement).getByRole("button", { name: "删除 聊天" }));
    expect(confirms[0]?.title).toBe("删除场景「聊天」？");
    confirms[0]?.onConfirm();
    await waitFor(() => {
      expect(backend.peek().scenes.map((s) => s.name)).toEqual(["GitHub"]);
    });
    // A refused change is a message, not a silent failure.
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("scenes: 不行"));
    await user.click(screen.getByRole("switch", { name: "启用 GitHub" }));
    await waitFor(() => {
      expect(notify).toHaveBeenCalledWith("出错了 · 不行", "danger");
    });
    backend.destroy();
  });

  it("regression: the output mode's notes say what the take really does: 整段输出 on a realtime model streams, 边说边输入 drops the scene's polish, and a fallback model standing in is named (docs/dictation.md §3.5, §12, §18.4)", async () => {
    // The review of 边说边输入 on realtime models, 2026-11-23: a scene's 整段输出 on a realtime model
    // ran as 边说边识别 and its 边说边输入 dropped the scene's AI 润色, without a word in the
    // editor; and with a whole-recording fallback model standing in, its note blamed the Live
    // preview model.
    const user = userEvent.setup();
    const streaming = {
      kind: "installed" as const,
      path: "/models/zipformer-stream-zh-en",
      installed_at: 1,
    };
    const backend = new MockBackend({
      ...NO_BUILTIN,
      scenes: [CHAT],
      providerKeys: [{ provider: "aliyun", kind: "asr" }],
      models: { "zipformer-stream-zh-en": streaming },
      settings: {
        engines: {
          ...defaultEngineSettings(),
          asr_provider: "aliyun",
          providers: { aliyun: { asr_model: "qwen-audio-3.1-asr-flash-streaming" } },
          asr_fallback: {
            enabled: true,
            models: [{ provider: "aliyun", model: "qwen-audio-3.1-asr-flash" }],
          },
        },
      },
    });
    renderScenes(backend);
    await user.click(await screen.findByRole("button", { name: "new scene" }));
    const editor = await screen.findByTestId("scene-editor");
    const mode = within(editor).getByRole("combobox", { name: "输出方式" });
    const refine = within(editor).getByRole("combobox", { name: "AI 润色" });
    await user.selectOptions(mode, "whole_take");
    expect(within(editor).getByTestId("scene-streamed-note")).toHaveTextContent(
      "所选的是实时识别模型：说话时就识别，与「边说边识别」相同；候补模型只能整段识别时按整段输出运行。",
    );
    // 边说边输入 with the polish on, by the scene or by the settings: it has no effect.
    await user.selectOptions(mode, "live_inject");
    expect(within(editor).queryByTestId("scene-streamed-note")).toBeNull();
    expect(within(editor).getByTestId("scene-refine-note")).toHaveTextContent(
      "「边说边输入」不进行润色，这个场景的 AI 润色不会生效",
    );
    await user.selectOptions(refine, "off");
    expect(within(editor).queryByTestId("scene-refine-note")).toBeNull();
    await user.selectOptions(refine, "on");
    expect(within(editor).getByTestId("scene-refine-note")).toBeInTheDocument();
    expect(within(editor).queryByTestId("scene-streaming-note")).toBeNull();
    // The realtime model's quota is used up and a whole-recording model stands in: the streaming
    // modes run whole for now, so the polish runs; the note names the fallback model.
    act(() => {
      backend.simulateQuotaExhausted("asr", NOW + 86_400_000);
    });
    await waitFor(() => {
      expect(within(editor).getByTestId("scene-streaming-note")).toHaveTextContent(
        "当前使用的候补模型不支持实时识别，按整段输出运行",
      );
    });
    expect(within(editor).queryByTestId("scene-refine-note")).toBeNull();
    backend.destroy();
  });

  it("regression: while a fallback model stands in for the built-in service, the editor says so under a streaming mode (docs/dictation.md §3.5, §18.4)", async () => {
    // The review of 2026-11-24: the note said the fallback model does not transcribe in real time,
    // untrue for a realtime fallback model.
    const user = userEvent.setup();
    const backend = new MockBackend({
      ...NO_BUILTIN,
      scenes: [CHAT],
      builtIn: {
        asr: { model: "Qwen/Qwen3-ASR-1.7B", key: true, preview: true },
        llm: { model: "qwen/qwen3.8-27b", key: true },
      },
      providerKeys: [{ provider: "volcengine_speech", kind: "asr" }],
      settings: {
        engines: {
          ...defaultEngineSettings(),
          asr_fallback: {
            enabled: true,
            models: [{ provider: "volcengine_speech", model: "volc.seedasr.sauc.duration" }],
          },
        },
      },
    });
    renderScenes(backend);
    await user.click(await screen.findByRole("button", { name: "new scene" }));
    const editor = await screen.findByTestId("scene-editor");
    await user.selectOptions(
      within(editor).getByRole("combobox", { name: "输出方式" }),
      "live_inject",
    );
    expect(within(editor).queryByTestId("scene-streaming-note")).toBeNull();
    act(() => {
      backend.simulateQuotaExhausted("asr", NOW + 86_400_000);
    });
    await waitFor(() => {
      expect(within(editor).getByTestId("scene-streaming-note")).toHaveTextContent(
        "候补模型正在代替内置服务，按整段输出运行",
      );
    });
    backend.destroy();
  });

  it("the desktop editor builds a scene from a typed id, a recent app and keywords, checks it before sending and keeps a refusal", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({
      ...NO_BUILTIN,
      scenes: [CHAT],
      history: [take(1, { id: "code", name: "Visual Studio Code" })],
    });
    const { notify } = renderScenes(backend);
    await user.click(screen.getByRole("button", { name: "new scene" }));
    const editor = await screen.findByTestId("scene-editor");
    // Nothing named yet: the missing name and app show after the first save.
    await user.click(screen.getByRole("button", { name: /保存/ }));
    expect(within(editor).getByText("请填写名称")).toBeInTheDocument();
    expect(within(editor).getByText("至少添加一个应用")).toBeInTheDocument();
    await user.type(within(editor).getByLabelText("名称"), "聊天");
    expect(within(editor).getByText("已有名为「聊天」的场景")).toBeInTheDocument();
    await user.clear(within(editor).getByLabelText("名称"));
    await user.type(within(editor).getByLabelText("名称"), "编程");

    await user.type(within(editor).getByRole("textbox", { name: "应用" }), " Terminal.EXE {Enter}");
    const recent = await within(editor).findByRole("group", { name: "最近的应用" });
    await user.click(within(recent).getByRole("button", { name: "Visual Studio Code" }));
    await user.click(within(recent).getByRole("button", { name: "Visual Studio Code" }));
    await user.click(within(recent).getByRole("button", { name: "Visual Studio Code" }));
    expect(within(editor).getByRole("list", { name: "应用" })).toHaveTextContent("terminalcode");
    await user.click(within(editor).getByRole("button", { name: "移除 terminal" }));
    await user.type(within(editor).getByRole("textbox", { name: "窗口标题关键词" }), "PR{Enter}");
    await user.type(within(editor).getByRole("textbox", { name: "窗口标题关键词" }), "pr{Enter}");
    expect(within(editor).getByRole("list", { name: "窗口标题关键词" })).toHaveTextContent("PR");
    await user.click(within(editor).getByRole("button", { name: "移除 PR" }));

    await user.selectOptions(
      within(editor).getByRole("combobox", { name: "输出方式" }),
      "streaming_final",
    );
    expect(within(editor).getByTestId("scene-streaming-note")).toBeInTheDocument();
    fireEvent.change(within(editor).getByLabelText("给 AI 的补充要求"), {
      target: { value: "长".repeat(MAX_SCENE_PROMPT_CHARS + 1) },
    });
    expect(within(editor).getByTestId("scene-prompt-count")).toHaveClass("text-danger");
    await user.click(screen.getByRole("button", { name: /保存/ }));
    expect(backend.peek().scenes.map((s) => s.name)).toEqual(["聊天"]);
    await user.clear(within(editor).getByLabelText("给 AI 的补充要求"));

    // Ctrl S saves.
    fireEvent.keyDown(editor, { key: "s", ctrlKey: true });
    await waitFor(() => {
      expect(backend.peek().scenes.find((s) => s.name === "编程")?.match).toEqual({
        apps: ["code"],
        title_contains: [],
      });
    });
    expect(notify).toHaveBeenCalledWith("已保存场景 · 编程");
    expect(screen.queryByTestId("scene-editor")).toBeNull();

    // An edit the core refuses stays open with the reason.
    await user.click(screen.getByRole("button", { name: "编辑 编程" }));
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("scenes: 名字太长"));
    await user.click(screen.getByRole("button", { name: /保存/ }));
    expect(await screen.findByTestId("scene-editor-error")).toHaveTextContent("名字太长");
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.queryByTestId("scene-editor")).toBeNull();
    backend.destroy();
  });

  it("with no history the editor says where recent apps come from", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ ...NO_BUILTIN, history: [] });
    vi.spyOn(backend, "recentApps").mockRejectedValueOnce(new Error("no history"));
    renderScenes(backend);
    expect(await screen.findByText("暂无场景")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "new scene" }));
    expect(await screen.findByText(/历史记录中暂无应用/)).toBeInTheDocument();
    backend.destroy();
  });

  it("on the phone the cards have no order, switches or matches, and the editor saves a scene with no app and no output mode", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone" });
    const recent = vi.spyOn(backend, "recentApps");
    const { notify, confirms } = renderScenes(backend, false);
    await waitFor(() => {
      expect(cards().length).toBe(backend.peek().scenes.length);
    });
    expect(cards().length).toBeGreaterThan(0);
    expect(screen.queryByRole("switch")).toBeNull();
    expect(screen.queryByRole("button", { name: /^上移/ })).toBeNull();
    expect(screen.queryByTestId("scene-match")).toBeNull();
    expect(screen.queryByTestId("scene-needs-apps")).toBeNull();
    // Built-in scenes start switched off; on the phone that does not dim them.
    expect(cards()[0]).not.toHaveClass("opacity-60");
    expect(
      within(cards()[0] as HTMLElement).getByTestId("scene-builtin-description"),
    ).toBeInTheDocument();

    // A built-in scene's term pack.
    const first = cards()[0] as HTMLElement;
    const name = first.getAttribute("aria-label") ?? "";
    await user.click(within(first).getByRole("button", { name: `查看 ${name} 的术语` }));
    const terms = screen.getByRole("dialog", { name: `${name} · 术语` });
    expect(within(terms).getByTestId("scene-terms-list").children.length).toBeGreaterThan(0);
    await user.click(within(terms).getByRole("button", { name: "关闭" }));

    // A built-in scene keeps its name; it saves and restores its defaults.
    await user.click(within(first).getByRole("button", { name: `编辑 ${name}` }));
    let editor = await screen.findByTestId("scene-editor");
    expect(within(editor).getByTestId("scene-builtin-name")).toHaveValue(name);
    expect(within(editor).queryByTestId("scene-editor-apps")).toBeNull();
    expect(within(editor).queryByRole("combobox", { name: "输出方式" })).toBeNull();
    await user.selectOptions(within(editor).getByRole("combobox", { name: "AI 润色" }), "off");
    await user.click(screen.getByRole("button", { name: /保存/ }));
    await waitFor(() => {
      expect(backend.peek().scenes[0]?.overrides.refine_enabled).toBe(false);
    });
    await user.click(
      within(cards()[0] as HTMLElement).getByRole("button", { name: `编辑 ${name}` }),
    );
    await user.click(await screen.findByTestId("scene-restore"));
    expect(confirms[0]?.title).toBe(`恢复「${name}」的默认设置？`);
    confirms[0]?.onConfirm();
    await waitFor(() => {
      expect(backend.peek().scenes[0]?.overrides.refine_enabled).toBeUndefined();
    });
    expect(notify).toHaveBeenCalledWith(`已恢复默认 · ${name}`);

    // A new scene names no application.
    await user.click(screen.getByRole("button", { name: "new scene" }));
    editor = await screen.findByTestId("scene-editor");
    expect(within(editor).queryByTestId("scene-editor-keywords")).toBeNull();
    await user.type(within(editor).getByLabelText("名称"), "会议纪要");
    await user.click(screen.getByRole("button", { name: /保存/ }));
    await waitFor(() => {
      expect(backend.peek().scenes.find((s) => s.name === "会议纪要")?.match.apps).toEqual([]);
    });
    expect(recent).not.toHaveBeenCalled();

    // A restore the core refuses is shown in the editor.
    await user.click(within(cards()[1] as HTMLElement).getByRole("button", { name: /^编辑 / }));
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("scenes: 恢复不了"));
    await user.click(await screen.findByTestId("scene-restore"));
    confirms[1]?.onConfirm();
    expect(await screen.findByTestId("scene-editor-error")).toHaveTextContent("恢复不了");
    backend.destroy();
  });

  it("regression: a saved built-in scene is named by its label, not its stored category", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const { notify } = renderScenes(backend);
    const chat = await screen.findByRole("article", { name: "即时聊天" });
    await user.click(within(chat).getByRole("button", { name: "编辑 即时聊天" }));
    await screen.findByTestId("scene-editor");
    await user.click(screen.getByRole("button", { name: /保存/ }));
    await waitFor(() => {
      expect(notify).toHaveBeenCalledWith("已保存场景 · 即时聊天");
    });
    expect(notify).not.toHaveBeenCalledWith("已保存场景 · chat");
    backend.destroy();
  });
});
