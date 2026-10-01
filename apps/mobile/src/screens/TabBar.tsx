import { Icon, type IconName, useI18n, useUiState } from "@voltip/ui";
import { useMobileShell } from "../app/shell";

/** The bottom tab bar on the tab roots (user decision 2026-10-01: the phone has its own settings):
 *  说话 is the welcome screen until a computer is paired, the device list after; 设置 the phone's
 *  own settings. */
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
      className="sticky bottom-0 flex shrink-0 border-t border-border bg-surface">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          aria-current={tab.active ? "page" : undefined}
          data-testid={`tab-${tab.id}`}
          onClick={tab.open}
          className={`flex flex-1 flex-col items-center gap-0.5 py-2 text-[11px] font-medium ${
            tab.active ? "text-accent" : "text-fg-muted"
          }`}>
          <Icon name={tab.icon} size={20} />
          {tab.label}
        </button>
      ))}
    </nav>
  );
}
