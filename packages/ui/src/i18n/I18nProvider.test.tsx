import { render, renderHook, screen } from "@testing-library/react";
import { CommandPalette } from "../components/CommandPalette";
import { Dialog } from "../components/Dialog";
import { Heatmap } from "../components/Heatmap";
import {
  LiveCaption,
  PILL_CAPTIONS,
  PILL_DEFAULT_LABEL,
  Pill,
  pillCaption,
} from "../components/Pill";
import { SafetyCodeView } from "../components/SafetyCodeView";
import { Sidebar } from "../components/Sidebar";
import { ThemeTile } from "../components/ThemeTile";
import { ToolbarSearch } from "../components/Toolbar";
import { I18nProvider, useI18n, useLocale, useT } from "./I18nProvider";

const CJK = /[一-鿿]/;

describe("I18nProvider", () => {
  it("defaults to zh-CN without a provider and follows the provider's locale", () => {
    const bare = renderHook(() => ({ t: useT(), locale: useLocale(), i18n: useI18n() }));
    expect(bare.result.current.locale).toBe("zh-CN");
    expect(bare.result.current.t("shell.nav.home")).toBe("首页");
    expect(bare.result.current.i18n.tag).toBe("zh-CN");
    const english = renderHook(() => ({ t: useT(), locale: useLocale() }), {
      wrapper: ({ children }) => <I18nProvider locale="en">{children}</I18nProvider>,
    });
    expect(english.result.current.locale).toBe("en");
    expect(english.result.current.t("shell.nav.home")).toBe("Home");
    expect(english.result.current.t("count.entries", { n: 2 })).toBe("2 entries");
  });

  it("mirrors the locale onto <html lang> only when asked", () => {
    document.documentElement.lang = "";
    const { rerender, unmount } = render(<I18nProvider locale="en">x</I18nProvider>);
    expect(document.documentElement.lang).toBe("");
    rerender(
      <I18nProvider locale="en" documentLang>
        x
      </I18nProvider>,
    );
    expect(document.documentElement.lang).toBe("en-US");
    rerender(
      <I18nProvider locale="zh-CN" documentLang>
        x
      </I18nProvider>,
    );
    expect(document.documentElement.lang).toBe("zh-CN");
    unmount();
  });

  it("regression: shared components render English copy under an English provider", () => {
    render(
      <I18nProvider locale="en">
        <Sidebar
          groups={[{ title: "Workspace", items: [{ id: "home", label: "Home", icon: "home" }] }]}
          activeId="home"
          onNavigate={() => undefined}
        />
        <Dialog
          open
          title="Delete?"
          onClose={() => undefined}
          actions={<button type="button">x</button>}>
          body
        </Dialog>
        <CommandPalette open items={[]} onClose={() => undefined} />
        <Pill state="armed" />
        <Pill state="listening" levels={[0.2]} />
        <Pill state="locked" levels={[0.2]} onStop={() => undefined} />
        <Pill state="error" onCopy={() => undefined} />
        <LiveCaption committed="" tail="" tier="failed" elapsed="00:01" engine="e" queueDepth={2} />
        <Heatmap values={[[0, 1]]} />
        <SafetyCodeView code={{ words: ["a", "b", "c", "d"], fingerprint: "AA" }} />
        <ThemeTile theme="warm" selected={false} onSelect={() => undefined} />
        <ToolbarSearch onSearch={() => undefined} />
      </I18nProvider>,
    );
    expect(screen.getByRole("navigation", { name: "Main navigation" })).toBeInTheDocument();
    expect(screen.getByLabelText("Core status")).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "Delete?" })).toHaveTextContent("Cancel");
    expect(screen.getByRole("dialog", { name: "Command menu" })).toHaveTextContent(
      "No matching command",
    );
    expect(screen.getByPlaceholderText("Type a command or page…")).toBeInTheDocument();
    expect(screen.getByText("0 results")).toBeInTheDocument();
    expect(
      screen.getByRole("img", { name: /Ready · hold to talk · Ctrl Alt Space · Local/ }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stop recording" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy text" })).toBeInTheDocument();
    expect(screen.getByText("Something went wrong")).toBeInTheDocument();
    expect(screen.getByText("2 queued")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Activity heatmap" })).toHaveTextContent("LessMore");
    expect(screen.getByRole("list", { name: "Safety code" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Warm" })).toBeInTheDocument();
    expect(screen.getByText("Search or type a command…")).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(CJK);
    // The default-locale constants keep the Chinese wording for callers that never mount a provider.
    expect(PILL_DEFAULT_LABEL.armed).toBe("待命 · 按住说话");
    expect(PILL_CAPTIONS.blocked).toMatch(/^blocked · /);
    expect(pillCaption("armed", (key) => key)).toBe("ui.pill.caption.armed");
  });
});
