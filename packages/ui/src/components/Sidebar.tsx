import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { Logo } from "./Logo";
import { Icon, type IconName } from "./Icon";
import { Lamp, type LampTone } from "./Lamp";

export interface SidebarItem {
  id: string;
  label: string;
  icon: IconName;
  count?: number | string;
}

export interface SidebarGroup {
  title: string;
  items: SidebarItem[];
}

export interface SidebarProps {
  groups: readonly SidebarGroup[];
  activeId: string;
  onNavigate: (id: string) => void;
  /** Items greyed out (onboarding keeps only the home item live). */
  disabledIds?: readonly string[];
  /** Brand lamp: green when the core is ready. */
  statusTone?: LampTone;
  footer?: readonly { id: string; icon: IconName; label: string; onClick: () => void }[];
  brand?: string;
  /** macOS keeps its native traffic lights over the top-left corner (`titleBarStyle: "Overlay"`);
   *  reserve their 72 px (x 12 + 2 × 20 pitch + 12 button + 8 breathing) so the brand does not
   *  sit under them. Keep in sync with `trafficLightPosition` in tauri.macos.conf.json. */
  trafficLights?: boolean;
  className?: string;
}

/** 224 px nav rail: 40 px brand row (part of the window drag strip), grouped items (36 px), footer icon buttons. */
export function Sidebar({
  groups,
  activeId,
  onNavigate,
  disabledIds = [],
  statusTone = "ok",
  footer = [],
  brand = "Voltip",
  trafficLights = false,
  className,
}: SidebarProps) {
  const t = useT();
  return (
    <nav
      aria-label={t("ui.sidebar.nav")}
      className={cx(
        "flex h-full w-56 shrink-0 flex-col border-r border-border bg-nav px-3 pb-4",
        className,
      )}>
      {/* Brand row: exactly 40 px, flush with the top edge, so it and the TitleBar form one
          continuous drag strip. `deep` lets the brand text drag too; the lamp is not clickable. */}
      <div
        data-tauri-drag-region="deep"
        data-testid="sidebar-brand"
        className={cx(
          "mb-6 flex h-10 shrink-0 items-center gap-2.5 select-none",
          trafficLights ? "pl-15 pr-2" : "px-2",
        )}>
        <Logo size={24} className="shrink-0" />
        <span className="text-[15px] font-semibold text-fg">{brand}</span>
        <Lamp tone={statusTone} size={6} className="ml-auto" label={t("ui.sidebar.coreStatus")} />
      </div>
      <div className="flex flex-1 flex-col gap-5">
        {groups.map((g) => (
          <div key={g.title}>
            <div className="mb-1 px-2 text-[11px] text-fg-subtle">{g.title}</div>
            <ul className="flex flex-col gap-0.5">
              {g.items.map((item) => {
                const active = item.id === activeId;
                const disabled = disabledIds.includes(item.id);
                return (
                  <li key={item.id}>
                    <button
                      type="button"
                      aria-current={active ? "page" : undefined}
                      disabled={disabled}
                      onClick={() => {
                        onNavigate(item.id);
                      }}
                      className={cx(
                        "flex h-9 w-full items-center gap-2.5 rounded-6 px-2 text-[13px] transition-colors",
                        active
                          ? "bg-nav-active font-medium text-fg"
                          : "text-fg-muted hover:bg-nav-active hover:text-fg",
                        disabled &&
                          "cursor-not-allowed opacity-40 hover:bg-transparent hover:text-fg-muted",
                      )}>
                      <Icon
                        name={item.icon}
                        size={16}
                        className={active ? "text-fg" : "text-fg-subtle"}
                      />
                      <span className="flex-1 truncate text-left">{item.label}</span>
                      {item.count !== undefined && (
                        <span className="mono text-[11px] text-fg-subtle">{item.count}</span>
                      )}
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </div>
      {footer.length > 0 && (
        <div className="mt-4 flex items-center gap-1 px-1">
          {footer.map((f) => (
            <button
              key={f.id}
              type="button"
              aria-label={f.label}
              title={f.label}
              onClick={f.onClick}
              className="flex h-7 w-7 items-center justify-center rounded-6 text-fg-subtle hover:bg-nav-active hover:text-fg">
              <Icon name={f.icon} size={15} />
            </button>
          ))}
        </div>
      )}
    </nav>
  );
}
