import { sampleUsage } from "@voltip/shared/mock";
import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";

// The built-in service's card on the phone's 语音模型 and AI 模型 pages (docs/dictation.md §27).
describe("the built-in service's usage on the phone's model pages", () => {
  it("says only what was used in the counting period, and the limit once the service sets one", async () => {
    // 2026-11-07, the user: today's usage shows the amount used, not 暂不限量 or 统计期间不设上限.
    const user = userEvent.setup();
    const { backend } = renderApp({
      initialScreen: "speech",
      mock: { usage: sampleUsage("count") },
    });
    const card = await screen.findByTestId("provider-asr-builtin");
    expect(within(card).getByTestId("provider-asr-usage")).toHaveTextContent(/^今日已用 23 分钟$/);
    await user.click(within(card).getByRole("button", { name: "内置服务" }));
    const drawer = await screen.findByTestId("builtin-usage-asr");
    expect(drawer).toHaveTextContent("今日已用 23 分钟");
    expect(drawer).not.toHaveTextContent(/暂不限量|不设上限/);
    // The service sets its limits: the next answer's report (or question) says so.
    act(() => {
      backend.simulateUsage({ quota: sampleUsage("ok").quota });
    });
    expect(within(card).getByTestId("provider-asr-usage")).toHaveTextContent(
      /^今日 23 \/ 60 分钟$/,
    );
    expect(drawer).toHaveTextContent("已用 23 / 60 分钟");
    // And back to counting.
    act(() => {
      backend.simulateUsage({ quota: sampleUsage("count").quota });
    });
    expect(within(card).getByTestId("provider-asr-usage")).toHaveTextContent(/^今日已用 23 分钟$/);
    expect(drawer).not.toHaveTextContent(/暂不限量|不设上限/);
  });
});
