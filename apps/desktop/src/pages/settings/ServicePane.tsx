import {
  BUILTIN_PRESETS,
  MIN_SERVE_PORT,
  type ServeSettings,
  errorText,
  presetLabel,
  sceneLabel,
} from "@voltip/shared";
import {
  Button,
  Input,
  LampText,
  Select,
  SettingsPane,
  SettingsRows,
  SettingsSection,
  StatusRow,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useState } from "react";
import { useShell } from "../../app/shell-context";

/** A port the service accepts (`voltip_core::MIN_SERVE_PORT`–65535). */
export function validPort(text: string): number | undefined {
  if (!/^\d{1,5}$/.test(text.trim())) return undefined;
  const port = Number.parseInt(text, 10);
  return port >= MIN_SERVE_PORT && port <= 65_535 ? port : undefined;
}

/** Settings · 本机服务 (docs/dictation.md §23.6): the switch, the status and the address other
 *  programs use, the port, the preset and scene of their `voltip` requests, and the token, which
 *  is copied to the clipboard by the core and never shown here. */
export function ServicePane() {
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const state = useUiState();
  const serve = state.settings.serve;
  const status = state.serve;
  const [portText, setPortText] = useState(String(serve.port));
  const [portTouched, setPortTouched] = useState(false);
  const port = validPort(portText);
  const fail = (e: unknown) => {
    shell.toast({ message: t("common.errorPrefix", { message: errorText(e) }), duration: 5000 });
  };
  const set = (patch: Partial<ServeSettings>) => {
    const next = { ...serve, ...patch };
    backend
      .invoke("settings_set_serve", {
        enabled: next.enabled,
        port: next.port,
        preset: next.preset ?? null,
        scene: next.scene ?? null,
      })
      .catch(fail);
  };
  const applyPort = () => {
    setPortTouched(true);
    if (port !== undefined && port !== serve.port) set({ port });
  };
  const rotate = () => {
    shell.confirm({
      title: t("settings.service.rotateConfirmTitle"),
      body: t("settings.service.rotateConfirmBody"),
      confirmLabel: t("settings.service.rotateConfirm"),
      tone: "danger",
      onConfirm: () => {
        backend.invoke("serve_rotate_token").then(() => {
          shell.toast({ message: t("settings.service.rotated"), duration: 3000 });
        }, fail);
      },
    });
  };
  const copy = () => {
    backend.invoke("serve_copy_token").then(() => {
      shell.toast({ message: t("settings.service.tokenCopied"), duration: 3000 });
    }, fail);
  };
  const statusText =
    status.phase === "running"
      ? t("settings.service.statusRunning")
      : status.phase === "failed"
        ? t("settings.service.statusFailed", { reason: status.error ?? "" })
        : t("settings.service.statusOff");
  const tone = status.phase === "running" ? "ok" : status.phase === "failed" ? "danger" : "off";
  const presetOptions = [
    { value: "", label: t("settings.service.presetFollow") },
    ...BUILTIN_PRESETS.map((id) => ({ value: id, label: presetLabel(id, state.presets, locale) })),
    ...state.presets.map((p) => ({ value: p.id, label: p.name })),
  ];
  const sceneOptions = [
    { value: "", label: t("settings.service.sceneNone") },
    ...state.scenes.map((scene) => ({ value: scene.id, label: sceneLabel(scene, locale) })),
  ];

  return (
    <SettingsPane
      title={t("settings.service.title")}
      lede={t("settings.service.lede")}
      data-testid="service-pane">
      <SettingsSection title={t("settings.service.serviceTitle")}>
        <SettingsRows>
          <StatusRow label={t("settings.service.enable")} help={t("settings.service.enableHelp")}>
            <Toggle
              checked={serve.enabled}
              ariaLabel={t("settings.service.enable")}
              onChange={(enabled) => {
                set({ enabled });
              }}
            />
          </StatusRow>
          <StatusRow label={t("settings.service.status")}>
            <LampText tone={tone} size="sm">
              <span
                data-testid="service-status"
                data-user-text={status.phase === "failed" ? "" : undefined}>
                {statusText}
              </span>
            </LampText>
          </StatusRow>
          {status.address !== undefined && status.phase === "running" && (
            <StatusRow
              label={t("settings.service.address")}
              help={t("settings.service.addressHelp")}>
              <span className="mono text-[12px] text-fg" data-testid="service-address">
                {status.address}
              </span>
            </StatusRow>
          )}
          <StatusRow
            label={t("settings.service.port")}
            help={t("settings.service.portHelp", { min: MIN_SERVE_PORT })}>
            <div className="flex items-start gap-2">
              <Input
                size="sm"
                mono
                inputMode="numeric"
                aria-label={t("settings.service.port")}
                className="w-24"
                value={portText}
                data-testid="service-port"
                error={
                  portTouched && port === undefined
                    ? t("settings.service.portInvalid", { min: MIN_SERVE_PORT })
                    : undefined
                }
                onChange={(e) => {
                  setPortText(e.target.value);
                  setPortTouched(false);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") applyPort();
                }}
              />
              <Button
                size="sm"
                variant="outline"
                data-testid="service-port-apply"
                disabled={port === serve.port}
                onClick={applyPort}>
                {t("settings.service.portApply")}
              </Button>
            </div>
          </StatusRow>
        </SettingsRows>
      </SettingsSection>

      <SettingsSection title={t("settings.service.processingTitle")}>
        <p className="mb-2 text-[12px] leading-5 text-fg-muted">
          {t("settings.service.processingLede")}
        </p>
        <SettingsRows>
          <StatusRow label={t("settings.service.preset")} help={t("settings.service.presetHelp")}>
            <Select
              label={t("settings.service.preset")}
              size="sm"
              value={serve.preset ?? ""}
              data-testid="service-preset"
              options={presetOptions}
              onChange={(value) => {
                set(value === "" ? { preset: undefined } : { preset: value });
              }}
            />
          </StatusRow>
          <StatusRow label={t("settings.service.scene")} help={t("settings.service.sceneHelp")}>
            <Select
              label={t("settings.service.scene")}
              size="sm"
              value={serve.scene ?? ""}
              data-testid="service-scene"
              options={sceneOptions}
              onChange={(value) => {
                set(value === "" ? { scene: undefined } : { scene: value });
              }}
            />
          </StatusRow>
        </SettingsRows>
      </SettingsSection>

      <SettingsSection title={t("settings.service.tokenTitle")}>
        <SettingsRows>
          <StatusRow label={t("settings.service.token")} help={t("settings.service.tokenHelp")}>
            <div className="flex items-center gap-2">
              <Button
                size="sm"
                variant="outline"
                icon="copy"
                data-testid="service-copy-token"
                onClick={copy}>
                {t("settings.service.copyToken")}
              </Button>
              <Button size="sm" variant="ghost" data-testid="service-rotate-token" onClick={rotate}>
                {t("settings.service.rotate")}
              </Button>
            </div>
          </StatusRow>
        </SettingsRows>
      </SettingsSection>
    </SettingsPane>
  );
}
