// 保存位置 (docs/dictation.md §10.9, issue #192): where the model library is, moving it through
// the shell's folder dialog with progress and 取消移动, 恢复默认 after a confirmation, how a move
// ended, and the model buttons that wait for a move.
import {
  MOCK_CHOSEN_MODELS_DIR,
  MOCK_MODEL_TICK_MS,
  MOCK_MODEL_TICKS,
  MOCK_MODELS_ROOT,
  MockBackend,
  sampleDevices,
} from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../../test/render";

type User = ReturnType<typeof userEvent.setup>;

const INSTALLED = {
  kind: "installed",
  path: `${MOCK_MODELS_ROOT}/qwen3-asr-0.6b`,
  installed_at: 1,
} as const;

/** The 本机识别 card's drawer, opened with a click as a user would; its location row. */
async function openLocation(user: User) {
  const card = await screen.findByTestId("provider-asr-local");
  if (card.getAttribute("data-open") !== "true") {
    const open = card.querySelector("button[aria-haspopup='dialog']");
    if (!(open instanceof HTMLElement)) throw new Error("no way to open the on-device card");
    await user.click(open);
  }
  return screen.findByTestId("models-location");
}

function modelCard(name: string) {
  return within(screen.getByTestId("local-models")).getByRole("article", { name });
}

function backend(options: ConstructorParameters<typeof MockBackend>[0] = {}) {
  return new MockBackend({
    devices: sampleDevices(1_758_700_000),
    now: () => 1_758_700_000_000,
    models: { "qwen3-asr-0.6b": INSTALLED },
    ...options,
  });
}

describe("ModelsLocation", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows the default folder, and moves the library into the folder the dialog picks", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const core = backend();
    const choose = vi.spyOn(core, "modelsLocationChoose");
    renderApp({ path: "/speech", backend: core });
    const row = await openLocation(user);
    expect(within(row).getByTestId("models-location-dir")).toHaveTextContent(MOCK_MODELS_ROOT);
    expect(within(row).getByText("默认")).toBeInTheDocument();
    expect(within(row).queryByTestId("models-location-reset")).toBeNull();

    await user.click(within(row).getByRole("button", { name: "更改位置…" }));
    expect(choose).toHaveBeenCalledWith("选择模型的保存位置");
    const moving = await within(row).findByTestId("models-location-moving");
    expect(moving).toHaveTextContent(`正在移动到 ${MOCK_CHOSEN_MODELS_DIR}`);
    // Nothing else writes into the library meanwhile.
    expect(within(row).getByRole("button", { name: "更改位置…" })).toBeDisabled();
    const balanced = modelCard("均衡");
    expect(within(balanced).getByRole("button", { name: "删除" })).toBeDisabled();
    expect(within(modelCard("轻量")).getByRole("button", { name: "下载" })).toBeDisabled();
    act(() => {
      vi.advanceTimersByTime(MOCK_MODEL_TICK_MS * MOCK_MODEL_TICKS);
    });
    await waitFor(() => {
      expect(within(row).getByTestId("models-location-outcome")).toHaveTextContent(
        "模型已移到新位置",
      );
    });
    expect(within(row).queryByTestId("models-location-moving")).toBeNull();
    expect(within(row).getByTestId("models-location-dir")).toHaveTextContent(
      MOCK_CHOSEN_MODELS_DIR,
    );
    expect(within(row).getByText("自定义")).toBeInTheDocument();
    expect(within(balanced).getByTestId("model-installed")).toHaveTextContent(
      `${MOCK_CHOSEN_MODELS_DIR}/qwen3-asr-0.6b`,
    );
    expect(within(balanced).getByRole("button", { name: "删除" })).toBeEnabled();

    // 恢复默认 asks first, then the library moves back.
    await user.click(within(row).getByTestId("models-location-reset"));
    const confirm = screen.getByRole("dialog", { name: "把模型移回默认位置？" });
    expect(confirm).toHaveTextContent(`已下载的模型会移回 ${MOCK_MODELS_ROOT}。`);
    await user.click(within(confirm).getByRole("button", { name: "移回默认位置" }));
    act(() => {
      vi.advanceTimersByTime(MOCK_MODEL_TICK_MS * MOCK_MODEL_TICKS);
    });
    await waitFor(() => {
      expect(within(row).getByTestId("models-location-dir")).toHaveTextContent(MOCK_MODELS_ROOT);
    });
    expect(within(row).getByText("默认")).toBeInTheDocument();
    expect(core.peek().models_location.custom).toBe(false);
  });

  it("a cancelled or failed move says the models stayed, and a closed dialog changes nothing", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const core = backend();
    renderApp({ path: "/speech", backend: core });
    const row = await openLocation(user);

    core.simulateModelsFolder(null);
    await user.click(within(row).getByRole("button", { name: "更改位置…" }));
    expect(within(row).queryByTestId("models-location-moving")).toBeNull();

    core.simulateModelsFolder(MOCK_CHOSEN_MODELS_DIR);
    await user.click(within(row).getByRole("button", { name: "更改位置…" }));
    await user.click(await within(row).findByTestId("models-location-cancel"));
    await waitFor(() => {
      expect(within(row).getByTestId("models-location-outcome")).toHaveTextContent(
        "已取消移动，模型仍在原来的位置",
      );
    });
    expect(within(row).getByTestId("models-location-dir")).toHaveTextContent(MOCK_MODELS_ROOT);

    core.simulateModelsMoveFails("所选位置空间不足：移动模型需要 791 MB 空闲空间，当前可用 512 MB");
    await user.click(within(row).getByRole("button", { name: "更改位置…" }));
    act(() => {
      vi.advanceTimersByTime(MOCK_MODEL_TICK_MS * MOCK_MODEL_TICKS);
    });
    await waitFor(() => {
      expect(within(row).getByTestId("models-location-outcome")).toHaveAttribute(
        "data-outcome",
        "failed",
      );
    });
    expect(within(row).getByTestId("models-location-outcome")).toHaveTextContent(
      "模型没有移动，仍在原来的位置：所选位置空间不足：移动模型需要 791 MB 空闲空间，当前可用 512 MB",
    );
    expect(within(row).getByTestId("models-location-dir")).toHaveTextContent(MOCK_MODELS_ROOT);
  });

  it("waits for a download, and says what an old folder in use means", async () => {
    const user = userEvent.setup();
    const core = backend({
      models: {
        "qwen3-asr-0.6b": INSTALLED,
        "sense-voice-small": {
          kind: "downloading",
          received: 1,
          total: 2,
          file: "model.int8.onnx",
        },
      },
      modelsLocation: {
        dir: MOCK_CHOSEN_MODELS_DIR,
        custom: true,
        last: { kind: "moved", dir: MOCK_CHOSEN_MODELS_DIR, deferred: 1 },
      },
    });
    renderApp({ path: "/speech", backend: core });
    const row = await openLocation(user);
    expect(within(row).getByRole("button", { name: "更改位置…" })).toBeDisabled();
    expect(within(row).getByTestId("models-location-reset")).toBeDisabled();
    expect(row).toHaveTextContent("模型下载期间不能更改位置");
    expect(within(row).getByTestId("models-location-outcome")).toHaveTextContent(
      "模型已移到新位置。旧位置有 1 个文件夹正在使用，下次启动 Voltip 时删除。",
    );
  });

  it("the phone has no library and so no location", () => {
    const core = new MockBackend({ role: "phone" });
    expect(core.peek().models_location.available).toBe(false);
    return expect(core.modelsLocationChoose("x")).rejects.toThrow("手机端不支持本地模型");
  });
});
