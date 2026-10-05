import { MockBackend } from "@voltip/shared/mock";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { BackendProvider, useBackend } from "../../backend/BackendProvider";
import { I18nProvider } from "../../i18n/I18nProvider";
import { RefineNotice } from "./RefineNotice";

/** Marks the provider's first state: an event before it is not folded in. */
function Loaded() {
  return useBackend().state === undefined ? null : <span data-testid="loaded" />;
}

async function renderNotice(
  backend: MockBackend,
  onOpen: () => void,
  locale: "zh-CN" | "en" = "zh-CN",
) {
  const view = render(
    <BackendProvider backend={backend}>
      <I18nProvider locale={locale}>
        <Loaded />
        <RefineNotice onOpen={onOpen} />
      </I18nProvider>
    </BackendProvider>,
  );
  await screen.findByTestId("loaded");
  return view;
}

describe("the built-in AI polish service's notice (docs/dictation.md §3.6)", () => {
  it("shows while the core reports one, leads to the AI models page, and closes through the core", async () => {
    const backend = new MockBackend();
    const opened: string[] = [];
    const { unmount } = await renderNotice(backend, () => opened.push("ai"));
    expect(screen.queryByTestId("refine-notice")).toBeNull();

    act(() => {
      backend.simulateRefineNotice("rate_limited");
    });
    expect(await screen.findByText("内置 AI 润色服务当前繁忙")).toBeInTheDocument();
    expect(screen.getByTestId("refine-notice")).toHaveTextContent("例如申请一个免费的 Groq 密钥");
    fireEvent.click(screen.getByRole("button", { name: "打开 AI 模型" }));
    expect(opened).toEqual(["ai"]);

    act(() => {
      backend.simulateRefineNotice("quota");
    });
    expect(await screen.findByText("内置 AI 润色服务的额度已用完")).toBeInTheDocument();

    const invoke = vi.spyOn(backend, "invoke");
    fireEvent.click(screen.getByRole("button", { name: "关闭" }));
    expect(invoke).toHaveBeenCalledWith("refine_notice_close");
    await waitFor(() => {
      expect(screen.queryByTestId("refine-notice")).toBeNull();
    });
    unmount();
  });

  it("reads in English", async () => {
    const backend = new MockBackend();
    const { unmount } = await renderNotice(backend, () => undefined, "en");
    act(() => {
      backend.simulateRefineNotice("rate_limited");
    });
    expect(await screen.findByText("The built-in AI polish service is busy")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open AI models" })).toBeInTheDocument();
    unmount();
  });
});
