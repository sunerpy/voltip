import { type OverlayPlacement, THEME_IDS, themeName } from "@voltip/shared";
import {
  ACCENT_IDS,
  Button,
  FONT_SIZE_MAX,
  FONT_SIZE_MIN,
  IconButton,
  Keycaps,
  LampText,
  Segmented,
  SettingsPane,
  SettingsRows,
  SettingsSection,
  StatusRow,
  ThemeTile,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useAppearance } from "../../app/appearance";
import { useShell } from "../../app/shell-context";
import { entryLatencyMs } from "../../features/history/stats";

/** Settings · Appearance: four theme tiles, follow-system, density, font size, overlay position, motion, preview. */
export function Appearance() {
  const { backend } = useBackend();
  const { settings, engines, history } = useUiState();
  // The preview strip draws the theme over live values: readiness, the hotkey, the last latency.
  const last = history[0];
  const appearance = useAppearance();
  const shell = useShell();
  const { t, locale } = useI18n();

  const setTheme = (theme: (typeof THEME_IDS)[number], followSystem: boolean) => {
    void backend.invoke("settings_set_theme", { theme, followSystem });
  };
  // The pill's placement is a core setting: the desktop shell places (or hides) the window.
  const setOverlay = (placement: OverlayPlacement) => {
    void backend.invoke("settings_set_overlay", { placement });
  };

  return (
    <SettingsPane title={t("settings.appearance.title")} lede={t("settings.appearance.lede")}>
      <SettingsSection
        title={t("settings.appearance.themeTitle")}
        description={t("settings.appearance.themeLede")}
        aside={
          settings.follow_system_theme ? (
            <span className="mono text-[11px] text-fg-subtle">
              {t("settings.appearance.followingSystem", {
                theme: themeName(appearance.resolvedTheme, locale),
              })}
            </span>
          ) : undefined
        }>
        <div
          role="radiogroup"
          aria-label={t("settings.appearance.themeGroup")}
          className="flex flex-wrap gap-4">
          {THEME_IDS.map((id) => (
            <ThemeTile
              key={id}
              theme={id}
              selected={appearance.resolvedTheme === id}
              disabled={settings.follow_system_theme}
              caption={id === "light" ? t("settings.appearance.lightCaption") : id}
              onSelect={(theme) => {
                const previous = settings.theme;
                setTheme(theme, false);
                shell.toast({
                  message: t("settings.appearance.themeSwitched", {
                    theme: themeName(theme, locale),
                  }),
                  duration: 5000,
                  action: {
                    label: t("settings.appearance.undo"),
                    onClick: () => {
                      setTheme(previous, false);
                    },
                  },
                });
              }}
            />
          ))}
        </div>
      </SettingsSection>

      <SettingsRows>
        <StatusRow
          label={t("settings.appearance.followSystem")}
          help={t("settings.appearance.followSystemHelp", {
            scheme: t(`settings.appearance.scheme.${appearance.systemDark ? "dark" : "light"}`),
          })}
          note={
            settings.follow_system_theme
              ? `prefers-color-scheme: ${appearance.systemDark ? "dark" : "light"}`
              : undefined
          }>
          <Toggle
            checked={settings.follow_system_theme}
            onChange={(next) => {
              setTheme(settings.theme, next);
            }}
            label={settings.follow_system_theme ? t("settings.appearance.followingNow") : undefined}
          />
        </StatusRow>
        <StatusRow
          label={t("settings.appearance.accent")}
          help={t("settings.appearance.accentHelp")}
          data-testid="accent-row">
          {/* ChatGPT's accent design (user decision 2026-09-29): each swatch paints the accent
              that choice gives the current theme, through the tokens (no colour literals here). */}
          <div
            role="radiogroup"
            aria-label={t("settings.appearance.accentGroup")}
            className="flex flex-wrap justify-end gap-1.5">
            {ACCENT_IDS.map((id) => {
              const chosen = appearance.local.accent === id;
              const name = t(`settings.appearance.accentName.${id}`);
              return (
                <button
                  key={id}
                  type="button"
                  role="radio"
                  aria-checked={chosen}
                  aria-label={name}
                  title={name}
                  data-accent={id}
                  data-theme={appearance.resolvedTheme}
                  onClick={() => {
                    appearance.setLocal({ accent: id });
                  }}
                  className={`flex h-7 w-7 items-center justify-center rounded-full bg-transparent ${chosen ? "hairline border-fg" : ""}`}>
                  <span className="h-[18px] w-[18px] rounded-full bg-accent" aria-hidden />
                </button>
              );
            })}
          </div>
        </StatusRow>
        <StatusRow
          label={t("settings.appearance.density")}
          help={t("settings.appearance.densityHelp")}>
          <Segmented
            label={t("settings.appearance.density")}
            value={appearance.local.density}
            onChange={(density) => {
              appearance.setLocal({ density });
            }}
            options={[
              { value: "compact", label: t("settings.appearance.compact") },
              { value: "default", label: t("settings.appearance.default") },
            ]}
          />
        </StatusRow>
        <StatusRow
          label={t("settings.appearance.fontSize")}
          help={t("settings.appearance.fontSizeHelp")}>
          <div className="flex items-center gap-1 rounded-6 bg-surface p-0.5 hairline">
            <IconButton
              icon="minus"
              label={t("settings.appearance.fontSmaller")}
              disabled={appearance.local.fontSizePx <= FONT_SIZE_MIN}
              onClick={() => {
                appearance.setLocal({ fontSizePx: appearance.local.fontSizePx - 1 });
              }}
            />
            <span className="mono w-14 text-center text-[13px] text-fg" data-testid="font-size">
              {appearance.local.fontSizePx} px
            </span>
            <IconButton
              icon="plus"
              label={t("settings.appearance.fontLarger")}
              disabled={appearance.local.fontSizePx >= FONT_SIZE_MAX}
              onClick={() => {
                appearance.setLocal({ fontSizePx: appearance.local.fontSizePx + 1 });
              }}
            />
          </div>
        </StatusRow>
        <StatusRow
          label={t("settings.appearance.overlay")}
          help={t("settings.appearance.overlayHelp")}>
          <Segmented
            label={t("settings.appearance.overlay")}
            value={settings.overlay}
            onChange={(placement) => {
              setOverlay(placement);
            }}
            options={[
              { value: "off", label: t("settings.appearance.overlayOff") },
              { value: "top", label: t("settings.appearance.overlayTop") },
              { value: "bottom", label: t("settings.appearance.overlayBottom") },
            ]}
          />
        </StatusRow>
        <StatusRow
          label={t("settings.appearance.reduceMotion")}
          help={t("settings.appearance.reduceMotionHelp")}
          note={appearance.systemReducedMotion ? t("settings.appearance.systemOn") : undefined}>
          <Toggle
            checked={appearance.local.reduceMotion || appearance.systemReducedMotion}
            disabled={appearance.systemReducedMotion}
            onChange={(reduceMotion) => {
              appearance.setLocal({ reduceMotion });
            }}
          />
        </StatusRow>
      </SettingsRows>

      <div
        className="flex h-16 items-center gap-6 rounded-10 bg-surface px-4 hairline"
        data-testid="preview-strip">
        <span className="eyebrow">{t("settings.appearance.preview")}</span>
        <LampText tone={engines.asr_ready ? "ok" : "warn"}>
          {engines.asr_ready ? t("settings.appearance.ready") : t("settings.appearance.notReady")}
        </LampText>
        <Keycaps keys={settings.hotkey.replaceAll("+", " ")} />
        <span className="mono ml-auto text-[11px] text-fg-muted" data-testid="preview-latency">
          {last === undefined
            ? t("settings.appearance.previewNoLatency")
            : t("settings.appearance.previewLatency", { ms: entryLatencyMs(last) })}
        </span>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            appearance.setLocal({
              density: "default",
              fontSizePx: 14,
              reduceMotion: false,
            });
            setTheme("light", false);
            setOverlay("bottom");
          }}>
          {t("settings.appearance.restore")}
        </Button>
      </div>
    </SettingsPane>
  );
}
