import { IconButton, Keycap, useI18n, useUiState } from "@voltip/ui";
import { settingsGroups } from "@voltip/shared/fixtures";
import { useCallback, useEffect } from "react";
import { useAppearance } from "../../app/appearance";
import { type SettingsSection, isSettingsSection, useRouter } from "../../app/router";
import { settingsReadouts } from "../../shell/page-meta";
import { Appearance } from "./Appearance";
import { AboutPane } from "./AboutPane";
import { General } from "./General";
import { Dictation } from "./Dictation";
import { Hotkey } from "./Hotkey";
import { Microphone } from "./Microphone";
import { PrivacyPane } from "./PrivacyPane";
import { ScenesPane } from "./scenes/ScenesPane";

const TITLE_ID = "vt-settings-title";

function tabId(section: string): string {
  return `vt-settings-tab-${section}`;
}

/** Settings as a modal over the page beneath: 200 px group nav on the left, a header with the
 *  group title, its readouts and 关闭 on the right, and a fluid scrolling form (max 720 px). The
 *  nav and the panes carry no English eyebrows or key suffixes: one language per locale (user
 *  2026-09-25, docs/frontend.md §6.4). 通用, 热键, 麦克风 (the input device and its test), 场景 (the
 *  scenes and the context switches,
 *  docs/dictation.md §18), 隐私 (what leaves the computer, the history switch and retention), 外观
 *  and 关于 (version, license, model sources, updates) are all backed by the core. 语音模型 and AI
 *  模型 are pages of the main layout (user feedback 2026-09-28). Esc and the scrim close back to
 *  the router's `background`; ↑/↓ move between groups. */
export function SettingsDialog({ section }: { section: SettingsSection }) {
  const { navigate, background } = useRouter();
  const state = useUiState();
  const appearance = useAppearance();
  const i18n = useI18n();
  const { t } = i18n;
  const close = useCallback(() => {
    navigate(background);
  }, [navigate, background]);

  // Opening the dialog (and moving between groups) lands focus on the active tab.
  useEffect(() => {
    document.getElementById(tabId(section))?.focus();
  }, [section]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // The palette handles its own Esc first and marks it; the hotkey recorder swallows Esc.
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.stopPropagation();
      close();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
    };
  }, [close]);

  const readouts = settingsReadouts(
    section,
    state,
    {
      resolvedTheme: appearance.resolvedTheme,
      density: appearance.local.density,
      fontSizePx: appearance.local.fontSizePx,
    },
    i18n,
  );

  const moveGroup = (delta: number) => {
    const idx = settingsGroups.findIndex((g) => g.id === section);
    const next = settingsGroups[(idx + delta + settingsGroups.length) % settingsGroups.length];
    if (next && isSettingsSection(next.id)) navigate({ name: "settings", section: next.id });
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center scrim"
      onClick={close}
      data-testid="settings-scrim">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={TITLE_ID}
        onClick={(e) => {
          e.stopPropagation();
        }}
        className="flex h-[min(660px,calc(100vh-48px))] w-[min(1120px,calc(100vw-48px))] overflow-hidden rounded-14 bg-surface hairline shadow-win">
        <nav className="flex w-[200px] shrink-0 flex-col border-r border-border bg-nav py-3">
          <div className="px-4 pb-3">
            <h2 id={TITLE_ID} className="text-[14px] font-semibold text-fg">
              {t("settings.title")}
            </h2>
          </div>
          <ul
            role="tablist"
            aria-label={t("settings.groupsLabel")}
            aria-orientation="vertical"
            className="flex flex-col"
            onKeyDown={(e) => {
              if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
              e.preventDefault();
              moveGroup(e.key === "ArrowDown" ? 1 : -1);
            }}>
            {settingsGroups.map((g) => {
              const active = g.id === section;
              return (
                <li key={g.id}>
                  <button
                    id={tabId(g.id)}
                    type="button"
                    role="tab"
                    aria-selected={active}
                    tabIndex={active ? 0 : -1}
                    onClick={() => {
                      navigate({
                        name: "settings",
                        section: isSettingsSection(g.id) ? g.id : "appearance",
                      });
                    }}
                    className={`flex h-9 w-full items-center px-4 text-[14px] outline-none hover:bg-nav-active focus-visible:bg-nav-active ${
                      active
                        ? "bg-nav-active font-semibold text-fg shadow-[inset_3px_0_0_var(--primary)]"
                        : "text-fg"
                    }`}>
                    <span>{t(`settings.group.${g.id}`)}</span>
                  </button>
                </li>
              );
            })}
          </ul>
          <div
            className="mono mt-auto px-4 text-[11px] text-fg-subtle"
            data-testid="settings-version">
            {state.app_version.length > 0 ? `Voltip ${state.app_version}` : "Voltip"}
          </div>
        </nav>
        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-12 shrink-0 items-center gap-4 border-b border-border px-6">
            <span className="text-[16px] font-semibold text-fg">
              {t(`settings.group.${section}`)}
            </span>
            {readouts.length > 0 && (
              <span
                className="mono truncate text-[11px] text-fg-subtle"
                data-testid="settings-readouts">
                {readouts.map((r) => `${r.label} ${r.value}`).join(" · ")}
              </span>
            )}
            <span className="mono ml-auto flex items-center gap-1.5 text-[10px] text-fg-subtle">
              <Keycap>Esc</Keycap> {t("settings.escClose")}
            </span>
            <IconButton icon="close" label={t("settings.close")} onClick={close} />
          </header>
          {/* One scroll area per group (the key): a group always opens at its top. A shared one
              kept the offset the previous group was read to, and WebKit could leave the panel
              empty after the content under that offset got shorter (user report 2026-09-29). */}
          <div
            key={section}
            role="tabpanel"
            aria-labelledby={tabId(section)}
            className="min-h-0 flex-1 overflow-auto p-6"
            data-testid="settings-content"
            data-section={section}>
            <div>
              {section === "general" && <General />}
              {section === "appearance" && <Appearance />}
              {section === "hotkey" && <Hotkey />}
              {section === "dictation" && <Dictation />}
              {section === "microphone" && <Microphone />}
              {section === "scene" && <ScenesPane />}
              {section === "privacy" && <PrivacyPane />}
              {section === "about" && <AboutPane />}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
