import { platformLabel, shortKey } from "@voltip/shared";
import {
  Button,
  Card,
  Input,
  Lamp,
  Readout,
  StatusRow,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useState } from "react";
import { Lede, PAGE, TOUCH, TOUCH_TOGGLE } from "../app/phone-ui";
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
    <div className={`${PAGE} min-h-full`}>
      {/* No eyebrow: the screen title above already says 本机 / This device. */}
      <Card className="flex flex-col gap-4">
        {!identity ? (
          <div className="flex items-center gap-2 text-[13px] text-fg-muted">
            <Lamp tone="idle" pulse /> {t("mobile.device.generating")}
          </div>
        ) : (
          <>
            {editing ? (
              <div className="flex flex-col gap-3">
                <Input
                  label={t("mobile.device.name")}
                  size="lg"
                  value={name}
                  maxLength={64}
                  onChange={(e) => {
                    setName(e.target.value);
                  }}
                  help={`${name.length} / 64`}
                />
                <div className="flex gap-2">
                  <Button
                    variant="primary"
                    className={TOUCH}
                    disabled={!dirty || name.trim().length === 0}
                    onClick={() => {
                      void backend.invoke("device_rename", { name: name.trim() });
                      setEditing(false);
                      shell.toast(t("mobile.device.renamed"));
                    }}>
                    {t("mobile.device.save")}
                  </Button>
                  <Button
                    variant="ghost"
                    className={TOUCH}
                    onClick={() => {
                      setName(identity.name);
                      setEditing(false);
                    }}>
                    {t("mobile.device.cancel")}
                  </Button>
                </div>
              </div>
            ) : (
              <div className="flex items-center justify-between gap-3">
                <div className="flex min-w-0 flex-col gap-0.5">
                  <div className="text-[11px] text-fg-subtle">{t("mobile.device.name")}</div>
                  <div className="text-[18px] font-semibold break-words text-fg" data-user-text>
                    {identity.name}
                  </div>
                </div>
                <Button
                  variant="ghost"
                  icon="edit"
                  className={`${TOUCH} -mr-2`}
                  onClick={() => {
                    setEditing(true);
                  }}>
                  {t("mobile.device.rename")}
                </Button>
              </div>
            )}
            <div className="grid grid-cols-2 gap-3 border-t border-border pt-4">
              <Readout
                label={t("mobile.device.platform")}
                value={platformLabel(identity.platform, locale)}
                size="sm"
              />
              <Readout label={t("mobile.device.keystore")} value={secret_backend} size="sm" />
            </div>
            <div className="flex flex-col gap-1">
              <div className="text-[11px] text-fg-subtle">{t("mobile.device.fingerprint")}</div>
              <div
                className="mono text-[15px] tracking-wider text-fg select-text"
                data-testid="fingerprint">
                {identity.fingerprint}
              </div>
              <div className="mono text-[11px] text-fg-subtle select-text">
                {t("mobile.device.publicKey", { key: shortKey(identity.public_key) })}
              </div>
            </div>
          </>
        )}
      </Card>
      <Lede>{t("mobile.device.note")}</Lede>
      <Card padding="none" className="px-4" data-testid="lan-discovery">
        <StatusRow label={t("mobile.device.lan")} help={t("mobile.device.lanHelp")}>
          <Toggle
            checked={settings.lan_discovery}
            ariaLabel={t("mobile.device.lan")}
            className={TOUCH_TOGGLE}
            onChange={(enabled) => {
              void backend.invoke("settings_set_lan_discovery", { enabled });
            }}
          />
        </StatusRow>
      </Card>
      <div className="mt-auto flex flex-col gap-2">
        <Button
          variant="primary"
          className={`${TOUCH} w-full`}
          disabled={!identity}
          onClick={() => {
            shell.go("pair");
          }}>
          {t("mobile.device.pair")}
        </Button>
        <Button
          variant="ghost"
          className={`${TOUCH} w-full`}
          onClick={() => {
            shell.go("devices");
          }}>
          {t("mobile.device.viewDevices")}
        </Button>
      </div>
    </div>
  );
}
