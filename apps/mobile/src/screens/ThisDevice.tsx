import { platformLabel, shortKey } from "@voltip/shared";
import {
  Button,
  Card,
  Input,
  Lamp,
  Readout,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useState } from "react";
import { useMobileShell } from "../app/shell";

export function ThisDevice() {
  const { backend } = useBackend();
  const { identity, secret_backend, settings } = useUiState();
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  const [name, setName] = useState(identity?.name ?? "");
  const [editing, setEditing] = useState(false);
  const dirty = identity !== null && name.trim() !== identity.name;

  return (
    <div className="flex h-full flex-col gap-4 p-4">
      {/* No eyebrow: the screen title above already says 本机 / This device. */}
      <Card className="flex flex-col gap-4">
        {!identity ? (
          <div className="flex items-center gap-2 text-[13px] text-fg-muted">
            <Lamp tone="idle" pulse /> {t("mobile.device.generating")}
          </div>
        ) : (
          <>
            {editing ? (
              <Input
                label={t("mobile.device.name")}
                value={name}
                maxLength={64}
                onChange={(e) => {
                  setName(e.target.value);
                }}
                help={`${name.length} / 64`}
              />
            ) : (
              <div className="flex items-center justify-between">
                <div>
                  <div className="text-[11px] text-fg-subtle">{t("mobile.device.name")}</div>
                  <div className="text-[20px] font-semibold text-fg">{identity.name}</div>
                </div>
                <Button
                  size="sm"
                  variant="ghost"
                  icon="edit"
                  onClick={() => {
                    setEditing(true);
                  }}>
                  {t("mobile.device.rename")}
                </Button>
              </div>
            )}
            {editing && (
              <div className="flex gap-2">
                <Button
                  size="sm"
                  variant="primary"
                  disabled={!dirty || name.trim().length === 0}
                  onClick={() => {
                    void backend.invoke("device_rename", { name: name.trim() });
                    setEditing(false);
                    shell.toast(t("mobile.device.renamed"));
                  }}>
                  {t("mobile.device.save")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    setName(identity.name);
                    setEditing(false);
                  }}>
                  {t("mobile.device.cancel")}
                </Button>
              </div>
            )}
            <div className="grid grid-cols-2 gap-3">
              <Readout
                label={t("mobile.device.platform")}
                value={platformLabel(identity.platform, locale)}
                size="sm"
              />
              <Readout label={t("mobile.device.keystore")} value={secret_backend} size="sm" />
            </div>
            <div>
              <div className="text-[11px] text-fg-subtle">{t("mobile.device.fingerprint")}</div>
              <div className="mono text-[15px] tracking-wider text-fg" data-testid="fingerprint">
                {identity.fingerprint}
              </div>
              <div className="mono mt-1 text-[11px] text-fg-subtle">
                {t("mobile.device.publicKey", { key: shortKey(identity.public_key) })}
              </div>
            </div>
          </>
        )}
      </Card>
      <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.device.note")}</p>
      <Card className="flex items-start justify-between gap-3" data-testid="lan-discovery">
        <div className="flex flex-col gap-1">
          <span className="text-[14px] font-medium text-fg">{t("mobile.device.lan")}</span>
          <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.device.lanHelp")}</p>
        </div>
        <Toggle
          checked={settings.lan_discovery}
          ariaLabel={t("mobile.device.lan")}
          className="mt-0.5"
          onChange={(enabled) => {
            void backend.invoke("settings_set_lan_discovery", { enabled });
          }}
        />
      </Card>
      <div className="mt-auto flex flex-col gap-2">
        <Button
          variant="primary"
          className="h-11 w-full text-[15px]"
          disabled={!identity}
          onClick={() => {
            shell.go("pair");
          }}>
          {t("mobile.device.pair")}
        </Button>
        <Button
          variant="ghost"
          className="h-11 w-full"
          onClick={() => {
            shell.go("devices");
          }}>
          {t("mobile.device.viewDevices")}
        </Button>
      </div>
    </div>
  );
}
