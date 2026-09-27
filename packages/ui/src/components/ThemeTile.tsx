import { type ThemeId, themeName } from "@voltip/shared";
import { cx } from "../cx";
import { useI18n } from "../i18n/I18nProvider";
import { Icon } from "./Icon";

export interface ThemeTileProps {
  theme: ThemeId;
  selected: boolean;
  onSelect: (theme: ThemeId) => void;
  disabled?: boolean;
  /** Mono caption right of the name, e.g. `light · 默认`. */
  caption?: string;
}

/** 148×104 mini shell painted with the target theme's own tokens (`data-theme` on the preview). */
export function ThemeTile({
  theme,
  selected,
  onSelect,
  disabled = false,
  caption,
}: ThemeTileProps) {
  const { locale } = useI18n();
  const name = themeName(theme, locale);
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      aria-label={name}
      disabled={disabled}
      onClick={() => {
        onSelect(theme);
      }}
      className={cx(
        "group flex w-[148px] flex-col gap-2 text-left",
        disabled && "cursor-not-allowed opacity-50",
      )}>
      <span
        data-theme={theme}
        className={cx(
          "relative flex h-[104px] w-[148px] overflow-hidden rounded-10 border bg-canvas transition-colors",
          selected
            ? "border-primary ring-1 ring-primary"
            : "border-border group-hover:border-fg-subtle",
        )}>
        <span className="h-full w-7 bg-nav" />
        <span className="flex flex-1 flex-col">
          <span className="h-2.5 w-full border-b border-border bg-surface" />
          <span className="flex flex-1 items-center justify-center p-2">
            <span className="flex w-full flex-col gap-1.5 rounded-[8px] border border-border bg-surface p-2">
              <span className="flex items-center gap-1.5">
                <span className="h-1.5 w-1.5 rounded-full bg-ok" />
                <span className="h-1 w-10 rounded-pill bg-fg" />
              </span>
              <span className="flex gap-[2px]">
                {[0, 1, 2, 3, 4].map((i) => (
                  <span
                    key={i}
                    className={cx("h-2 w-2 rounded-[1px]", i < 3 ? "bg-accent" : "bg-led-off")}
                  />
                ))}
              </span>
              <span className="h-1 w-6 rounded-pill bg-fg-subtle" />
            </span>
          </span>
        </span>
        {selected && (
          <span className="absolute top-1.5 right-1.5 flex h-3.5 w-3.5 items-center justify-center rounded-full bg-primary text-primary-fg">
            <Icon name="check" size={9} strokeWidth={3} />
          </span>
        )}
      </span>
      <span className="flex w-full items-baseline justify-between">
        <span className="text-[13px] text-fg">{name}</span>
        <span className="mono text-[11px] text-fg-subtle">{caption ?? theme}</span>
      </span>
    </button>
  );
}
