import { type MirrorProfile, mirrorStateText, presetLabel, sceneLabel } from "@voltip/shared";
import { EmptyState, useBackend, useI18n, useNow, useUiState } from "@voltip/ui";
import { type ReactNode, useEffect, useState } from "react";
import { useMobileShell } from "../app/shell";

/** The settings the copy of `desktop` holds (`mirror_profile`), asked again whenever the copy
 *  changes: `undefined` until the answer, `null` while none arrived. */
function useProfile(desktop: string): MirrorProfile | null | undefined {
  const { backend } = useBackend();
  const copy = useUiState().mirrors.find((m) => m.desktop === desktop);
  const revision = `${copy?.state}:${copy?.synced_at_ms}`;
  const [answer, setAnswer] = useState<{ desktop: string; profile: MirrorProfile | null }>();
  useEffect(() => {
    let live = true;
    backend.mirrorProfile(desktop).then(
      (profile) => {
        if (live) setAnswer({ desktop, profile });
      },
      () => {
        if (live) setAnswer({ desktop, profile: null });
      },
    );
    return () => {
      live = false;
    };
    // The copy changed (new settings arrived, or it was deleted): ask again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [backend, desktop, revision]);
  return answer?.desktop === desktop ? answer.profile : undefined;
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2" aria-label={title}>
      <h2 className="px-1 text-[12px] font-medium text-fg-subtle">{title}</h2>
      <div className="rounded-10 bg-surface px-4 hairline">{children}</div>
    </section>
  );
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-3 border-b border-border py-2.5 last:border-b-0">
      <dt className="shrink-0 text-[13px] text-fg-muted">{label}</dt>
      <dd className="min-w-0 text-right text-[13px] break-words text-fg">{children}</dd>
    </div>
  );
}

/** One item of a list section: a title and an optional detail line. */
function Item({ title, detail }: { title: string; detail?: string }) {
  return (
    <li className="flex flex-col gap-0.5 border-b border-border py-2.5 last:border-b-0">
      <span className="text-[13px] text-fg" data-user-text>
        {title}
      </span>
      {detail !== undefined && detail.length > 0 && (
        <span className="text-[12px] break-words text-fg-muted" data-user-text>
          {detail}
        </span>
      )}
    </li>
  );
}

function List({
  items,
  none,
}: {
  items: { key: string; title: string; detail?: string }[];
  none: string;
}) {
  if (items.length === 0) return <p className="py-2.5 text-[13px] text-fg-muted">{none}</p>;
  return (
    <ul>
      {items.map((i) => (
        <Item
          key={i.key}
          title={i.title}
          {...(i.detail === undefined ? {} : { detail: i.detail })}
        />
      ))}
    </ul>
  );
}

/** 设置 › 电脑 › a computer's settings (docs/dictation.md §20.8; user decision 2026-10-02):
 *  read-only — its language and theme as set there (the phone keeps its own), its recognition and
 *  AI polish, its presets, dictionary, rules and scenes, as the copy last received them. */
export function ComputerSettings() {
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  const now = useNow();
  const desktop = shell.param ?? "";
  const copy = useUiState().mirrors.find((m) => m.desktop === desktop);
  const profile = useProfile(desktop);
  const name = copy?.name ?? "";

  if (profile === undefined) return null;
  return (
    <div className="flex flex-col gap-5 p-4" data-testid="phone-computer-settings">
      <div className="flex flex-col gap-1 px-1">
        <p className="text-[12px] leading-5 text-fg-muted">{t("mirror.settings.lede", { name })}</p>
        {copy !== undefined && (
          <p className="text-[12px] text-fg-subtle" data-state={copy.state}>
            {mirrorStateText(copy, Math.floor(now / 1000), locale)}
          </p>
        )}
      </div>
      {profile === null ? (
        <EmptyState compact title={t("mirror.settings.none")} />
      ) : (
        <>
          <Section title={t("mirror.settings.look")}>
            <dl>
              <Fact label={t("mirror.settings.locale")}>
                {t(`settings.general.locale.${profile.locale}`)}
              </Fact>
              <Fact label={t("mirror.settings.theme")}>
                {profile.follow_system_theme
                  ? t("theme.followSystem")
                  : t(`theme.name.${profile.theme}`)}
              </Fact>
            </dl>
          </Section>
          <Section title={t("mirror.settings.speech")}>
            <dl>
              <Fact label={t("mirror.settings.speechModel")}>
                {t("mobile.settings.serviceDetail", {
                  provider: t(`engines.provider.${profile.asr_provider}`),
                  model: profile.asr_model.length > 0 ? profile.asr_model : "—",
                })}
              </Fact>
            </dl>
          </Section>
          <Section title={t("mirror.settings.polish")}>
            <dl>
              <Fact label={t("mirror.settings.polishState")}>
                {profile.refine_enabled
                  ? t("mirror.settings.polishOn")
                  : t("mirror.settings.polishOff")}
              </Fact>
              <Fact label={t("mirror.settings.polishModel")}>
                {t("mobile.settings.serviceDetail", {
                  provider:
                    profile.llm_provider === undefined
                      ? "—"
                      : t(`engines.provider.${profile.llm_provider}`),
                  model: profile.refine_model.length > 0 ? profile.refine_model : "—",
                })}
              </Fact>
              <Fact label={t("mirror.settings.preset")}>
                <span data-user-text>{presetLabel(profile.preset, profile.presets, locale)}</span>
              </Fact>
            </dl>
          </Section>
          <Section title={t("mirror.settings.presets")}>
            <List
              none={t("mirror.settings.none_items")}
              items={profile.presets.map((p) => ({ key: p.id, title: p.name, detail: p.prompt }))}
            />
          </Section>
          <Section title={t("mirror.settings.dictionary")}>
            <List
              none={t("mirror.settings.none_items")}
              items={profile.dictionary.map((d) => ({
                key: d.id,
                title: d.term,
                ...(d.heard_as.length > 0
                  ? { detail: t("mirror.settings.heardAs", { terms: d.heard_as.join("、") }) }
                  : {}),
              }))}
            />
          </Section>
          <Section title={t("mirror.settings.rules")}>
            <List
              none={t("mirror.settings.none_items")}
              items={profile.rules.map((r) => ({
                key: r.id,
                title: r.name,
                detail: `${r.pattern} → ${r.replacement}`,
              }))}
            />
          </Section>
          <Section title={t("mirror.settings.scenes")}>
            <List
              none={t("mirror.settings.none_items")}
              items={profile.scenes.map((s) => ({ key: s.id, title: sceneLabel(s, locale) }))}
            />
          </Section>
        </>
      )}
    </div>
  );
}
