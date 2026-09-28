import { screen } from "@testing-library/react";

describe("the entry point", () => {
  it("regression: a first launch on a fresh profile opens the home page, not the setup guide", async () => {
    // What the window starts with: no stored answers, the home route, the element Vite mounts on.
    window.localStorage.clear();
    window.history.replaceState(null, "", "/");
    document.body.innerHTML = '<div id="root"></div>';
    await import("./main");
    expect(await screen.findByTestId("page-home")).toBeInTheDocument();
    expect(window.location.pathname).toBe("/");
    expect(screen.queryByRole("list", { name: "步骤" })).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: /设置向导/ })).not.toBeInTheDocument();
  });
});
