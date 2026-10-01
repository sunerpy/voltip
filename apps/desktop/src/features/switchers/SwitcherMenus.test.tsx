import type { AudioDevice, ModelInstallState } from "@voltip/shared";
import { MOCK_AUDIO_DEVICES } from "@voltip/shared/mock";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";

const INSTALLED: ModelInstallState = { kind: "installed", path: "/models/x", installed_at: 1 };

/** The open menu of the trigger `testId` (Menu renders it as `<testId>-menu`). */
function menuOf(testId: string): HTMLElement {
  return screen.getByTestId(`${testId}-menu`);
}

// User request 2026-09-30 (items 2–5): the speech model, the AI 润色 model and the microphone are
// switched in place, from the title bar and from the home page, without opening a page.
describe("switcher menus", () => {
  it("regression: the title bar's 语音模型 menu switches to an installed local model and back to the built-in service", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ mock: { models: { "qwen3-asr-0.6b": INSTALLED } } });
    const trigger = await screen.findByTestId("title-bar-speech");
    expect(trigger).toHaveAccessibleName("语音模型：Qwen3-ASR-1.7B");
    await user.click(trigger);
    await user.click(
      within(menuOf("title-bar-speech")).getByRole("menuitemradio", {
        name: "均衡 · Qwen3-ASR 0.6B",
      }),
    );
    await waitFor(() => {
      expect(backend.peek().settings.engines).toMatchObject({
        asr_provider: "local",
        local_model: "qwen3-asr-0.6b",
      });
    });
    await waitFor(() => {
      expect(screen.getByTestId("title-bar-readout-badge")).toHaveTextContent("本机");
    });
    expect(trigger).not.toHaveAccessibleName("语音模型：Qwen3-ASR-1.7B");
    await user.click(trigger);
    const local = within(menuOf("title-bar-speech")).getByRole("menuitemradio", {
      name: "均衡 · Qwen3-ASR 0.6B",
    });
    expect(local).toHaveAttribute("aria-checked", "true");
    await user.click(
      within(menuOf("title-bar-speech")).getByRole("menuitemradio", { name: "Qwen3-ASR-1.7B" }),
    );
    await waitFor(() => {
      expect(backend.peek().settings.engines.asr_provider).toBe("builtin");
    });
    await waitFor(() => {
      expect(trigger).toHaveAccessibleName("语音模型：Qwen3-ASR-1.7B");
    });
  });

  it("regression: the 麦克风 menu reads the devices again each time it opens, records from the one chosen, and switches the recording source", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    const devices = vi.spyOn(backend, "audioDevices");
    const trigger = await screen.findByTestId("title-bar-mic");
    await waitFor(() => {
      expect(trigger).toHaveAccessibleName("麦克风：Fifine K669");
    });
    // A USB microphone plugged in while the window was open shows on the next opening.
    const plugged: AudioDevice[] = [
      ...MOCK_AUDIO_DEVICES.map((d) => ({ ...d })),
      { id: "Blue Yeti", name: "Blue Yeti USB Microphone", is_default: false },
    ];
    devices.mockResolvedValue(plugged);
    await user.click(trigger);
    const yeti = await within(menuOf("title-bar-mic")).findByRole("menuitemradio", {
      name: "Blue Yeti",
    });
    expect(
      within(menuOf("title-bar-mic")).getByRole("menuitemradio", {
        name: "系统默认（Fifine K669）",
      }),
    ).toHaveAttribute("aria-checked", "true");
    await user.click(yeti);
    await waitFor(() => {
      expect(backend.peek().settings.microphone).toBe("Blue Yeti");
    });
    await waitFor(() => {
      expect(trigger).toHaveAccessibleName("麦克风：Blue Yeti");
    });
    // What a take records: the title bar's menu carries the 录音来源 group too.
    await user.click(trigger);
    await user.click(within(menuOf("title-bar-mic")).getByRole("menuitemradio", { name: "混合" }));
    await waitFor(() => {
      expect(backend.peek().settings.recording.source).toBe("mixed");
    });
    expect(devices.mock.calls.length).toBeGreaterThanOrEqual(2);
  });

  // Overflow check 2026-10-01: in English at 960 px the setup guide's longer title leaves the
  // title bar's microphone too little room, and its short name was cut with nothing to read it by.
  it("regression: the title bar's 麦克风 and AI 润色模型 buttons read whole on hover", async () => {
    renderApp({ mock: { engines: { refine_model: "" } } });
    const mic = await screen.findByTestId("title-bar-mic");
    await waitFor(() => {
      expect(mic).toHaveAttribute("title", "Fifine K669 USB Microphone");
    });
    const polish = screen.getByTestId("polish-model");
    expect(polish).toHaveAttribute("title", polish.textContent);
  });

  it("regression: the AI 润色模型 menu beside the switch picks a model of a provider that is set up, and the home card's model opens the same menu", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ mock: { providerKeys: [{ provider: "groq", kind: "asr" }] } });
    const trigger = await screen.findByTestId("polish-model");
    expect(trigger).toHaveAccessibleName("AI 润色模型：qwen3.8-27b");
    await user.click(trigger);
    const menu = menuOf("polish-model");
    // OpenAI, SiliconFlow, DeepSeek, Ollama and the custom endpoint have no key: not listed.
    expect(within(menu).queryByRole("group", { name: "OpenAI" })).toBeNull();
    await user.click(within(menu).getByRole("menuitemradio", { name: "gpt-oss-20b" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.llm_provider).toBe("groq");
    });
    expect(backend.peek().settings.engines.providers?.groq?.llm_model).toMatch(/gpt-oss-20b$/);
    await waitFor(() => {
      expect(trigger).toHaveAccessibleName("AI 润色模型：gpt-oss-20b");
    });
    // The home page's 润色模型 is the same menu: back to the built-in service from there.
    const home = screen.getByTestId("home-refine-model");
    expect(home).toHaveAccessibleName("AI 润色模型：gpt-oss-20b");
    await user.click(home);
    const builtin = within(menuOf("home-refine-model")).getAllByRole("menuitemradio", {
      name: "qwen3.8-27b",
    })[0];
    if (builtin === undefined) throw new Error("the built-in service lists its model");
    await user.click(builtin);
    await waitFor(() => {
      expect(backend.peek().settings.engines.llm_provider).toBe("builtin");
    });
  });
});
