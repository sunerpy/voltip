import {
  ACTIVATIONS,
  activationLabel,
  type InjectPreflight,
  PERMISSIONS,
  PERMISSION_POLL_MAX_ERRORS,
  type Permission,
  type PermissionState,
  dictationPhaseLabel,
  nothingToGrant,
  onboardingGate,
  platformLabel,
  viaLabel,
} from "@voltip/shared";
import {
  Badge,
  Banner,
  Button,
  Card,
  Icon,
  Keycaps,
  Lamp,
  LampText,
  LedMeter,
  Table,
  type TableColumn,
  Textarea,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useEffect, useState } from "react";
import { inTextField, onControl, usePageShortcuts, withCommand } from "../app/page-shortcuts";
import { useRouter } from "../app/router";
import { copyWithToast, useShell } from "../app/shell-context";
import { levelFraction, useAudioMeter } from "../features/audio/useAudioMeter";
import { useChosenMicrophone } from "../features/audio/useMicrophoneTest";
import { useDictation, useTickingNow } from "../features/dictation/useDictation";
import { usePermissions } from "../features/permissions/usePermissions";
import { shortModel } from "../shell/page-meta";
import {
  type EngineChoice,
  type EngineDraft,
  OnboardingEngineStep,
  engineDraftProblem,
  engineSettingsFor,
  initialChoice,
  initialDraft,
} from "./OnboardingEngine";

/** The bundle identifier of `src-tauri/tauri.conf.json` (a test keeps them equal). */
export const MACOS_BUNDLE_ID = "dev.voltip.desktop";
/** Forget this app's Accessibility grant so macOS asks again (the remedy for a grant that does not
 *  stick after an unsigned or re-installed build). */
export const MACOS_TCC_RESET = `tccutil reset Accessibility ${MACOS_BUNDLE_ID}`;
const STEPS = ["permissions", "hotkey", "engine", "trial"] as const;
const TRIAL_METER_SEGMENTS = 24;

export interface OnboardingProps {
  step: number;
}

/** One row of the permission table: the permission and what the OS currently says. */
interface PermissionRow {
  id: Permission;
  state: PermissionState | undefined;
}

function permissionTone(state: PermissionState): "ok" | "danger" | "neutral" {
  if (state === "granted") return "ok";
  if (state === "denied") return "danger";
  return "neutral";
}

/** The setup guide (设置 › 通用; it no longer opens by itself on the first launch): a 560-wide wizard card with a shared stepper and footer. Step 1
 *  polls `permissions_status` every second (docs/dictation.md §15.1) and blocks "continue" on a
 *  denied required permission; step 2 reads the core's hotkey and the shell's registration; step 3
 *  writes the recognition provider (`settings_set_engines`, plus `provider_key_set` for a key); step 4
 *  runs a real dictation through the core, shows the result and the injection preflight (§15.3). */
export function Onboarding({ step }: OnboardingProps) {
  const { navigate } = useRouter();
  const shell = useShell();
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const state = useUiState();
  const dictation = useDictation();
  const current = Math.min(4, Math.max(1, step));
  const hotkey = state.settings.hotkey;
  const hotkeyStatus = state.hotkey;
  const platform = state.identity?.platform;
  const engines = state.engines;
  const permissions = usePermissions(backend, current === 1);
  const permissionReport = permissions.report;
  const gate = permissionReport ? onboardingGate(permissionReport) : [];
  const [preflight, setPreflight] = useState<InjectPreflight | undefined>(undefined);

  const [edges, setEdges] = useState<"waiting" | "pressed" | "passed" | "skipped">("waiting");
  // Step 3 follows the saved settings until the user edits it: the guide may mount before
  // `core_state` has arrived.
  const [choiceEdit, setChoice] = useState<EngineChoice | undefined>(undefined);
  const choice = choiceEdit ?? initialChoice(engines);
  const [draftEdit, setEngineDraft] = useState<EngineDraft | undefined>(undefined);
  const engineDraft = draftEdit ?? initialDraft(state.settings.engines, engines);
  // The trial step: the meter only runs while the recorder is open; the last finished result stays
  // in the box after the core's phase has returned to idle.
  const trialMeter = useAudioMeter(current === 4 && dictation.listening, useChosenMicrophone());
  const now = useTickingNow(current === 4 && dictation.listening);
  const trialPhase = dictation.phase;
  const latestDone =
    trialPhase.phase === "done"
      ? {
          text: trialPhase.text,
          via: viaLabel(trialPhase.via, locale),
          refined: trialPhase.refined,
        }
      : undefined;
  // Remember the last finished result across the core's return to idle (derived state adjusted
  // during render, keyed on the phase object the core sent).
  const [trial, setTrial] = useState<{ phase: unknown; result: typeof latestDone }>({
    phase: undefined,
    result: undefined,
  });
  if (latestDone && trial.phase !== trialPhase) setTrial({ phase: trialPhase, result: latestDone });
  const trialResult = latestDone ?? trial.result;

  const go = (next: number) => {
    navigate({ name: "onboarding", step: next });
  };
  const finish = () => {
    shell.toast({ message: t("onboarding.finished"), duration: 3000 });
    navigate({ name: "home" });
  };

  // The trial step asks once whether an injection would land (Windows integrity levels / secure
  // desktop); other hosts answer unchecked and the line stays hidden.
  useEffect(() => {
    if (current !== 4) return;
    let disposed = false;
    backend.injectPreflight().then(
      (p) => {
        if (!disposed) setPreflight(p);
      },
      () => {
        if (!disposed) setPreflight(undefined);
      },
    );
    return () => {
      disposed = true;
    };
  }, [current, backend]);

  // The system hotkey reports both edges through the shell (`hotkey.pressed`); in a browser
  // preview nothing is registered, so the same chord typed into this window counts too.
  useEffect(() => {
    if (current !== 2) return;
    return backend.on((event) => {
      if (event.type !== "hotkey") return;
      if (event.pressed) setEdges((prev) => (prev === "waiting" ? "pressed" : prev));
      else setEdges((prev) => (prev === "pressed" ? "passed" : prev));
    });
  }, [current, backend]);
  useEffect(() => {
    if (current !== 2 || edges === "passed" || edges === "skipped") return;
    const wanted = hotkey.split("+");
    const key = wanted[wanted.length - 1] ?? "";
    const isKey = (e: KeyboardEvent) =>
      key === "Space"
        ? e.key === " " || e.code === "Space"
        : e.key.toUpperCase() === key.toUpperCase();
    const isCombo = (e: KeyboardEvent) =>
      isKey(e) &&
      wanted.includes("Ctrl") === e.ctrlKey &&
      wanted.includes("Alt") === e.altKey &&
      wanted.includes("Shift") === e.shiftKey &&
      wanted.includes("Meta") === e.metaKey;
    const onDown = (e: KeyboardEvent) => {
      if (isCombo(e)) {
        e.preventDefault();
        setEdges("pressed");
      }
    };
    const onUp = (e: KeyboardEvent) => {
      if (isKey(e)) setEdges((prev) => (prev === "pressed" ? "passed" : prev));
    };
    window.addEventListener("keydown", onDown);
    window.addEventListener("keyup", onUp);
    return () => {
      window.removeEventListener("keydown", onDown);
      window.removeEventListener("keyup", onUp);
    };
  }, [current, edges, hotkey]);

  const engineProblem = engineDraftProblem(choice, engineDraft, engines, t);
  const saveEngine = () => {
    void backend.invoke("settings_set_engines", {
      engines: engineSettingsFor(choice, state.settings.engines, engineDraft, engines),
    });
    const key = engineDraft.key.trim();
    if (choice === "provider" && key.length > 0)
      void backend.invoke("provider_key_set", {
        provider: engineDraft.provider,
        kind: "asr",
        value: key,
      });
    go(4);
  };
  const trialLabel = dictationPhaseLabel(trialPhase, now, locale);
  const platformText = platform ? platformLabel(platform, locale) : t("onboarding.platformUnknown");
  const backendText =
    hotkeyStatus.backend.length > 0 ? hotkeyStatus.backend : t("onboarding.backendNotReported");

  const permissionRows: PermissionRow[] = PERMISSIONS.map((id) => ({
    id,
    state: permissionReport?.[id],
  }));
  const permissionColumns: TableColumn<PermissionRow>[] = [
    {
      id: "perm",
      header: t("onboarding.permission.column.permission"),
      mono: false,
      cell: (r) => ({
        type: "two",
        primary: t(`onboarding.permission.row.${r.id}.name`),
        secondary: t(`onboarding.permission.row.${r.id}.purpose`),
      }),
    },
    {
      id: "status",
      header: t("onboarding.permission.column.status"),
      width: 112,
      mono: false,
      cell: (r) => {
        if (r.state === undefined)
          return (
            <span className="text-[12px] text-fg-subtle">
              {t("onboarding.permission.status.reading")}
            </span>
          );
        const text = t(`onboarding.permission.status.${r.state}`);
        return (
          <span data-testid={`permission-${r.id}`} data-state={r.state}>
            {r.state === "not_applicable" ? (
              <span className="text-[12px] text-fg-subtle">{text}</span>
            ) : (
              <Badge tone={permissionTone(r.state)}>{text}</Badge>
            )}
          </span>
        );
      },
    },
    {
      id: "action",
      header: t("onboarding.permission.column.action"),
      width: 136,
      align: "right",
      mono: false,
      cell: (r) =>
        r.state === "denied" || r.state === "not_determined" ? (
          <Button
            size="sm"
            variant={gate.includes(r.id) ? "primary" : "outline"}
            onClick={() => {
              void permissions.request(r.id);
            }}>
            {t("onboarding.permission.request")}
          </Button>
        ) : (
          <span className="text-fg-subtle">—</span>
        ),
    },
  ];
  const permissionHint =
    gate.length === 0
      ? undefined
      : gate.includes("microphone")
        ? t("onboarding.permission.micDenied")
        : t("onboarding.permission.axMissing");
  // The footer's keys: Enter is the step's primary button (when it is enabled), Shift Enter goes
  // back, Esc is 稍后. Fields and focused buttons keep their own Enter / Esc.
  const primary = (): (() => void) | undefined => {
    if (current === 1) return gate.length > 0 ? undefined : () => go(2);
    if (current === 2) return () => go(3);
    if (current === 3) return engineProblem === undefined ? saveEngine : undefined;
    return finish;
  };
  usePageShortcuts((e) => {
    if (inTextField(e.target) || withCommand(e) || e.altKey) return false;
    if (e.key === "Enter" && e.shiftKey) {
      if (current <= 1) return false;
      go(current - 1);
      return true;
    }
    if (e.key === "Enter") {
      const action = onControl(e.target) ? undefined : primary();
      action?.();
      return action !== undefined;
    }
    if (e.key === "Escape" && !e.shiftKey) {
      finish();
      return true;
    }
    return false;
  });
  const preflightText =
    preflight?.checked && current === 4
      ? t(`onboarding.trial.preflight.${preflight.decision}`, {
          process: preflight.target_process ?? t("onboarding.trial.preflight.unknownProcess"),
        })
      : undefined;

  return (
    <div className="flex justify-center p-6">
      <Card padding="none" radius={14} className="flex w-[560px] flex-col">
        <div className="p-6 pb-0">
          <ol className="flex gap-4" aria-label={t("onboarding.stepsLabel")}>
            {STEPS.map((key, i) => {
              const n = i + 1;
              const done = n < current;
              const active = n === current;
              return (
                <li
                  key={key}
                  className="flex flex-1 flex-col gap-2"
                  aria-current={active ? "step" : undefined}>
                  <span
                    className={`h-0.5 w-full ${done ? "bg-accent" : active ? "bg-primary" : "bg-border"}`}
                  />
                  <span className="flex items-center gap-2 text-[13px]">
                    {done ? (
                      <span className="flex h-3.5 w-3.5 items-center justify-center rounded-full bg-accent text-accent-fg">
                        <Icon name="check" size={9} strokeWidth={3} />
                      </span>
                    ) : (
                      <span className="mono text-[11px] text-fg-subtle">0{n}</span>
                    )}
                    <span className={active ? "text-fg" : "text-fg-muted"}>
                      {t(`onboarding.steps.${key}`)}
                    </span>
                  </span>
                </li>
              );
            })}
          </ol>

          <div className="mt-5 flex items-start justify-between">
            <div>
              <div className="eyebrow">
                {current === 1 && t("onboarding.eyebrow.permissions", { platform: platformText })}
                {current === 2 && t("onboarding.eyebrow.hotkey", { backend: backendText })}
                {current === 3 && t("onboarding.eyebrow.engine")}
                {current === 4 && t("onboarding.eyebrow.trial")}
              </div>
              <h2 className="mt-1 text-[20px] font-semibold text-fg">
                {current === 1 && t("onboarding.title.permissions")}
                {current === 2 && t("onboarding.title.hotkey")}
                {current === 3 && t("onboarding.title.engine")}
                {current === 4 && t("onboarding.title.trial")}
              </h2>
              <p className="mt-1 text-[13px] text-fg-muted">
                {current === 1 && t("onboarding.lede.permissions")}
                {current === 2 && t("onboarding.lede.hotkey")}
                {current === 3 && t("onboarding.lede.engine")}
                {current === 4 && t("onboarding.lede.trial")}
              </p>
            </div>
            {current === 1 && (
              <span className="flex items-center gap-2">
                <span
                  className="mono text-[11px] text-fg-subtle"
                  data-testid="permission-poll"
                  data-stopped={permissions.stopped}>
                  {permissions.stopped
                    ? t("onboarding.permission.pollStopped", { n: PERMISSION_POLL_MAX_ERRORS })
                    : t("onboarding.permission.polling")}
                </span>
                <Button variant="text" size="sm" onClick={permissions.recheck}>
                  {t("onboarding.permission.recheck")}
                </Button>
              </span>
            )}
          </div>

          <div className="mt-4 min-h-[300px]">
            {current === 1 && (
              <>
                {permissions.stopped && (
                  <Banner
                    tone="danger"
                    marker="bar"
                    className="mb-4"
                    title={
                      <span className="eyebrow text-danger">
                        {t("onboarding.permission.readErrorTitle")}
                      </span>
                    }
                    actions={
                      <Button size="sm" variant="ghost" onClick={permissions.recheck}>
                        {t("onboarding.permission.recheck")}
                      </Button>
                    }>
                    {t("onboarding.permission.readError", { message: permissions.error ?? "" })}
                  </Banner>
                )}
                {permissionReport && nothingToGrant(permissionReport) && (
                  <div data-testid="nothing-to-grant">
                    <Banner tone="ok" marker="lamp" className="mb-4">
                      {t("onboarding.permission.nothingToGrant", { platform: platformText })}
                    </Banner>
                  </div>
                )}
                <Card padding="none" className="overflow-hidden">
                  <Table
                    label={t("onboarding.permission.table")}
                    columns={permissionColumns}
                    rows={permissionRows}
                    rowKey={(r) => r.id}
                  />
                </Card>
                {/* The TCC reset is a macOS-only remedy for grants that do not stick (an unsigned or
                    re-installed build); never show it elsewhere. */}
                {platform === "macos" && (
                  <Banner
                    tone="warn"
                    marker="bar"
                    className="mt-4"
                    title={
                      <span className="eyebrow text-warning">
                        {t("onboarding.permission.unsignedTitle")}
                      </span>
                    }
                    actions={
                      <Button
                        size="sm"
                        variant="ghost"
                        icon="copy"
                        onClick={() => {
                          void copyWithToast(
                            shell,
                            MACOS_TCC_RESET,
                            t("onboarding.permission.copiedCommand"),
                          );
                        }}>
                        {t("onboarding.permission.copyCommand")}
                      </Button>
                    }>
                    {t("onboarding.permission.unsignedBody")}
                    <span className="mono"> {MACOS_TCC_RESET}</span>
                    {t("onboarding.permission.unsignedAfter")}
                  </Banner>
                )}
              </>
            )}

            {current === 2 && (
              <div className="flex flex-col gap-4">
                <Card padding="sm" className="flex items-center justify-between">
                  <div>
                    <div className="eyebrow">{t("onboarding.hotkey.inForce")}</div>
                    <div className="mt-2 flex items-center gap-3">
                      <Keycaps keys={hotkey} />
                      <span
                        className="text-[12px] text-fg-muted"
                        data-testid="onboarding-hotkey-status">
                        {hotkeyStatus.error
                          ? hotkeyStatus.error
                          : hotkeyStatus.registered === hotkey
                            ? t("onboarding.hotkey.registered")
                            : t("onboarding.hotkey.pending")}
                      </span>
                    </div>
                  </div>
                  <span
                    className="mono text-[11px] text-fg-subtle"
                    data-testid="onboarding-hotkey-backend">
                    {platform ? platformLabel(platform, locale) : "—"} ·{" "}
                    {hotkeyStatus.backend.length > 0
                      ? hotkeyStatus.backend
                      : t("onboarding.hotkey.backendPending")}
                  </span>
                </Card>
                <div>
                  <div className="eyebrow mb-2">{t("onboarding.hotkey.triggerModes")}</div>
                  <div className="flex flex-wrap gap-2" data-testid="onboarding-activations">
                    {ACTIVATIONS.map((mode) => (
                      <span
                        key={mode}
                        data-active={mode === state.settings.activation || undefined}
                        className="inline-flex h-7 items-center gap-1.5 rounded-6 bg-surface px-2.5 text-[12px] hairline">
                        <Lamp tone={mode === state.settings.activation ? "ok" : "idle"} size={6} />
                        {activationLabel(mode, locale)}
                      </span>
                    ))}
                  </div>
                  <p className="mt-2 text-[12px] text-fg-muted">
                    {t("onboarding.hotkey.triggerNote")}
                  </p>
                </div>
                <Card padding="sm" data-testid="edge-monitor" data-edges={edges}>
                  <div className="eyebrow mb-2">{t("onboarding.hotkey.edgeTitle")}</div>
                  <div className="flex items-center justify-between">
                    <LampText
                      tone={edges === "passed" ? "ok" : edges === "pressed" ? "accent" : "idle"}
                      mono
                      pulse={edges === "pressed"}>
                      {edges === "waiting" && t("onboarding.hotkey.edge.idle")}
                      {edges === "pressed" && t("onboarding.hotkey.edge.pressed")}
                      {edges === "passed" && t("onboarding.hotkey.edgePassed")}
                      {edges === "skipped" && t("onboarding.hotkey.edge.skipped")}
                    </LampText>
                    <span className="mono text-[11px] text-fg-subtle">
                      {edges === "passed"
                        ? t("onboarding.hotkey.edgesDone")
                        : t("onboarding.hotkey.edgesWaiting")}
                    </span>
                  </div>
                  <p className="mt-2 text-[12px] text-fg-muted">
                    {t("onboarding.hotkey.edgeNote", { hotkey })}
                  </p>
                  <div className="mt-2 flex justify-end">
                    <Button
                      variant="text"
                      size="sm"
                      onClick={() => {
                        setEdges("skipped");
                      }}>
                      {t("onboarding.hotkey.skip")}
                    </Button>
                  </div>
                </Card>
              </div>
            )}

            {current === 3 && (
              <OnboardingEngineStep
                choice={choice}
                onChoice={setChoice}
                draft={engineDraft}
                onDraft={setEngineDraft}
              />
            )}

            {current === 4 && (
              <div className="flex flex-col gap-3">
                <Textarea
                  label={t("onboarding.trial.label")}
                  rows={4}
                  readOnly
                  value={trialResult?.text ?? ""}
                  placeholder={t("onboarding.trial.placeholder", {
                    hotkey,
                    sentence: t("onboarding.trial.sentence"),
                  })}
                />
                <Card
                  padding="sm"
                  className="flex items-center justify-between gap-4"
                  data-testid="trial-status"
                  data-phase={trialPhase.phase}>
                  <LedMeter
                    level={trialMeter.frame ? levelFraction(trialMeter.frame.rms_dbfs) : 0}
                    segments={TRIAL_METER_SEGMENTS}
                    size="sm"
                    disabled={!dictation.listening}
                  />
                  <LampText
                    tone={trialLabel.tone}
                    mono
                    pulse={dictation.listening || dictation.processing}>
                    {trialPhase.phase === "idle" ? t("onboarding.trial.idle") : trialLabel.text}
                  </LampText>
                </Card>
                {preflightText !== undefined && (
                  <span
                    className="mono text-[11px] text-fg-muted"
                    data-testid="trial-preflight"
                    data-decision={preflight?.decision}>
                    {preflightText}
                  </span>
                )}
                <span className="text-[11px] text-fg-subtle" data-testid="trial-note">
                  {trialResult
                    ? t("onboarding.trial.heard", {
                        refined: trialResult.refined
                          ? t("onboarding.trial.refined")
                          : t("onboarding.trial.notRefined"),
                        via: trialResult.via,
                      })
                    : t("onboarding.trial.pending", {
                        model: shortModel(engines.asr_model),
                        provider: t(`engines.provider.${engines.asr_provider}`),
                        refine: engines.refine_enabled
                          ? t("onboarding.trial.pendingRefine", {
                              model: shortModel(engines.refine_model),
                            })
                          : t("onboarding.trial.pendingNoRefine"),
                      })}
                </span>
                <span className="flex items-center gap-2">
                  {dictation.listening ? (
                    <Button variant="danger" icon="stop" onClick={dictation.stop}>
                      {t("onboarding.trial.stop")}
                    </Button>
                  ) : (
                    <Button
                      variant="outline"
                      icon="mic"
                      disabled={dictation.processing}
                      onClick={dictation.start}>
                      {dictation.processing
                        ? t("onboarding.trial.processing")
                        : trialResult
                          ? t("onboarding.trial.again")
                          : t("onboarding.trial.start")}
                    </Button>
                  )}
                  {dictation.listening && (
                    <Button variant="text" size="sm" onClick={dictation.cancel}>
                      {t("onboarding.trial.cancel")}
                    </Button>
                  )}
                </span>
              </div>
            )}
          </div>
        </div>

        <div className="mt-6 flex items-center justify-between gap-4 border-t border-border px-6 py-4">
          <span className="mono min-w-0 truncate text-[11px] text-fg-subtle">
            {platformText} ·{" "}
            {hotkeyStatus.backend.length > 0
              ? hotkeyStatus.backend
              : t("onboarding.backendNotReportedLong")}
          </span>
          <div className="flex shrink-0 items-center gap-3">
            {current === 1 && permissionHint !== undefined && (
              <span className="text-[12px] text-fg-muted" data-testid="permission-hint">
                {permissionHint}
              </span>
            )}
            {current === 3 && engineProblem !== undefined && (
              <span className="max-w-[240px] truncate text-[11px] text-fg-muted">
                {engineProblem}
              </span>
            )}
            <Button variant="text" onClick={finish}>
              {t("onboarding.footer.later")}
            </Button>
            {current > 1 && (
              <Button
                onClick={() => {
                  go(current - 1);
                }}>
                {t("onboarding.footer.previous")}
              </Button>
            )}
            {current === 3 ? (
              <Button
                variant="primary"
                disabled={engineProblem !== undefined}
                title={engineProblem}
                onClick={saveEngine}>
                {t("onboarding.footer.saveContinue")}
              </Button>
            ) : current < 4 ? (
              <Button
                variant="primary"
                disabled={current === 1 && gate.length > 0}
                title={current === 1 && gate.length > 0 ? permissionHint : undefined}
                onClick={() => {
                  go(current + 1);
                }}>
                {t("onboarding.footer.continue")}
              </Button>
            ) : (
              <Button variant="primary" onClick={finish}>
                {t("onboarding.footer.finish")}
              </Button>
            )}
          </div>
        </div>
      </Card>
    </div>
  );
}
