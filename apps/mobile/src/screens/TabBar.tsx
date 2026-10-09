import { Icon, type IconName, cx, useI18n, useUiState } from "@voltip/ui";
import { useMobileShell } from "../app/shell";

/** The bottom tab bar on the tab roots (user decision 2026-10-01: the phone has its own settings
 *  and history): 说话 is the welcome screen until a computer is paired, the device list after;
 *  记录 what the phone recognised itself; 设置 the phone's own settings. The tab in view is marked
 *  as the desktop's sidebar marks its page: ink text on the nav-active shade, the others muted.
 *  The bar runs under the navigation bar. */
export function TabBar() {
  const shell = useMobileShell();
  const { t } = useI18n();
  const { devices } = useUiState();
  const tabs: { id: string; icon: IconName; label: string; active: boolean; open: () => void }[] = [
    {
      id: "talk",
      icon: "mic",
      label: t("mobile.tab.talk"),
      active: shell.screen === "welcome" || shell.screen === "devices",
      open: () => {
        shell.go(devices.length > 0 ? "devices" : "welcome");
      },
    },
    {
      id: "history",
      icon: "history",
      label: t("mobile.tab.history"),
      active: shell.screen === "history",
      open: () => {
        shell.go("history");
      },
    },
    {
      id: "settings",
      icon: "settings",
      label: t("mobile.tab.settings"),
      active: shell.screen === "settings",
      open: () => {
        shell.go("settings");
      },
    },
  ];
  return (
    <nav
      aria-label={t("mobile.tab.label")}
      className="flex shrink-0 border-t border-border bg-surface pb-[env(safe-area-inset-bottom)]">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          aria-current={tab.active ? "page" : undefined}
          data-testid={`tab-${tab.id}`}
          onClick={tab.open}
          className={cx(
            "group flex h-14 flex-1 flex-col items-center justify-center gap-0.5 text-[11px] font-medium transition-colors",
            tab.active ? "text-fg" : "text-fg-muted active:text-fg",
          )}>
          <span
            className={cx(
              "flex h-7 w-14 items-center justify-center rounded-pill transition-colors",
              tab.active ? "bg-nav-active" : "group-active:bg-inset",
            )}>
            <Icon name={tab.icon} size={18} />
          </span>
          {tab.label}
        </button>
      ))}
    </nav>
  );
}
