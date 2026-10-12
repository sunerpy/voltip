import {
  DEFAULT_NO_PROXY,
  MAX_NO_PROXY_BYTES,
  MAX_PROXY_PASSWORD_BYTES,
  MAX_PROXY_USERNAME_BYTES,
  PROXY_MODES,
  type ProxyMode,
  type ProxyProblem,
  type ProxySettings,
  checkProxySettings,
  errorText,
  proxyArgs,
  proxyPasswordProblem,
  proxyProblemMessage,
} from "@voltip/shared";
import {
  Button,
  Input,
  Segmented,
  SettingsPane,
  SettingsRows,
  SettingsSection,
  StatusRow,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useEffect, useState } from "react";
import { useShell } from "../../app/shell-context";

/** What the manual proxy's form holds; the password is only ever typed, never shown. */
interface Draft {
  server: string;
  username: string;
  password: string;
  noProxy: string;
}

function draftOf(proxy: ProxySettings): Draft {
  return {
    server: proxy.server ?? "",
    username: proxy.username ?? "",
    password: "",
    // Saved as `a,b`; shown as `a, b`.
    noProxy: proxy.no_proxy.split(",").filter(Boolean).join(", "),
  };
}

const MODE_LABEL = {
  system: "settings.network.modeSystem",
  off: "settings.network.modeOff",
  manual: "settings.network.modeManual",
} as const satisfies Record<ProxyMode, string>;

const MODE_HELP = {
  system: "settings.network.systemHelp",
  off: "settings.network.offHelp",
  manual: "settings.network.manualHelp",
} as const satisfies Record<ProxyMode, string>;

/** Settings · 网络 (docs/dictation.md §24): the proxy every connection goes through — follow the
 *  system (the default, with the system's proxy shown and read again when the pane opens), none,
 *  or one set here. Choosing 跟随系统 or 不使用代理 applies at once; the manual proxy is checked
 *  as the core checks it and applies on 保存. Its password goes to the keychain and never comes
 *  back: the form only says whether one is saved. */
export function NetworkPane() {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const { settings, network } = useUiState();
  const proxy = settings.proxy;
  // The mode shown: the saved one, or 手动设置 while a first manual proxy is being filled in.
  const [mode, setMode] = useState<ProxyMode>(proxy.mode);
  const [draft, setDraft] = useState<Draft>(() => draftOf(proxy));
  // Problems are shown once a save was tried, then as the fields change.
  const [tried, setTried] = useState(false);
  // What was saved (tidied by the core) replaces the form's text and mode as it arrives.
  const saved = JSON.stringify(proxy);
  const [seen, setSeen] = useState(saved);
  if (seen !== saved) {
    setSeen(saved);
    setMode(proxy.mode);
    setDraft((d) => ({ ...draftOf(proxy), password: d.password }));
  }

  useEffect(() => {
    void backend.invoke("network_refresh");
  }, [backend]);

  const fail = (e: unknown) => {
    shell.toast({ message: t("common.errorPrefix", { message: errorText(e) }), duration: 5000 });
  };
  const send = (next: ProxySettings, password: string | null) =>
    backend.invoke("settings_set_proxy", proxyArgs(next, password));

  const choose = (next: ProxyMode) => {
    setMode(next);
    setTried(false);
    // A manual proxy needs a server before it can apply; until then the form waits for 保存.
    // A first manual proxy starts with the local network in its no-proxy list.
    if (next === "manual" && proxy.server === null && draft.server === "" && draft.noProxy === "")
      setDraft({ ...draft, noProxy: DEFAULT_NO_PROXY });
    if (next === proxy.mode || (next === "manual" && proxy.server === null)) return;
    send({ ...proxy, mode: next }, null).catch(fail);
  };

  const typed: ProxySettings = {
    mode: "manual",
    server: draft.server,
    username: draft.username,
    no_proxy: draft.noProxy,
  };
  const checked = checkProxySettings(typed);
  const problem: ProxyProblem | undefined =
    "problem" in checked ? checked.problem : proxyPasswordProblem(draft.password, draft.username);
  const shown = (field: ProxyProblem["field"]) =>
    tried && problem?.field === field ? proxyProblemMessage(problem, t) : undefined;
  const changed =
    proxy.mode !== "manual" ||
    draft.password !== "" ||
    JSON.stringify(draftOf(proxy)) !== JSON.stringify({ ...draft, password: "" });

  const save = () => {
    setTried(true);
    if ("problem" in checked || problem !== undefined) return;
    send(checked.proxy, draft.password === "" ? null : draft.password).then(() => {
      setDraft((d) => ({ ...d, password: "" }));
      setTried(false);
      shell.toast({ message: t("settings.network.saved"), duration: 3000 });
    }, fail);
  };
  const revert = () => {
    setDraft(draftOf(proxy));
    setTried(false);
    setMode(proxy.mode);
  };
  const clearPassword = () => {
    send(proxy, "").then(() => {
      shell.toast({ message: t("settings.network.passwordCleared"), duration: 3000 });
    }, fail);
  };

  return (
    <SettingsPane
      title={t("settings.network.title")}
      lede={t("settings.network.lede")}
      data-testid="network-pane">
      <SettingsSection title={t("settings.network.proxyTitle")}>
        <SettingsRows>
          <StatusRow label={t("settings.network.mode")} help={t(MODE_HELP[mode])}>
            <Segmented
              label={t("settings.network.mode")}
              value={mode}
              onChange={choose}
              options={PROXY_MODES.map((value) => ({ value, label: t(MODE_LABEL[value]) }))}
            />
          </StatusRow>
          {mode === "system" && (
            <StatusRow
              label={t("settings.network.systemProxy")}
              help={
                network.system_proxy === undefined ? t("settings.network.systemNone") : undefined
              }
              data-testid="network-system">
              <div className="flex items-center gap-2">
                <span
                  className="mono truncate text-[12px] text-fg"
                  data-testid="network-system-proxy">
                  {network.system_proxy ?? t("settings.network.systemProxyNone")}
                </span>
                <Button
                  size="sm"
                  variant="ghost"
                  icon="refresh"
                  data-testid="network-refresh"
                  onClick={() => {
                    backend.invoke("network_refresh").catch(fail);
                  }}>
                  {t("settings.network.refresh")}
                </Button>
              </div>
            </StatusRow>
          )}
          {mode === "manual" && (
            <>
              <StatusRow
                label={t("settings.network.server")}
                help={t("settings.network.serverHelp")}>
                <Input
                  size="sm"
                  mono
                  className="w-[420px]"
                  aria-label={t("settings.network.server")}
                  placeholder="http://127.0.0.1:7890"
                  autoComplete="off"
                  spellCheck={false}
                  value={draft.server}
                  error={shown("server")}
                  data-testid="network-server"
                  onChange={(e) => {
                    setDraft({ ...draft, server: e.target.value });
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") save();
                  }}
                />
              </StatusRow>
              <StatusRow
                label={t("settings.network.username")}
                help={t("settings.network.usernameHelp")}>
                <Input
                  size="sm"
                  className="w-[420px]"
                  aria-label={t("settings.network.username")}
                  autoComplete="off"
                  spellCheck={false}
                  maxLength={MAX_PROXY_USERNAME_BYTES}
                  value={draft.username}
                  error={shown("username")}
                  data-testid="network-username"
                  onChange={(e) => {
                    setDraft({ ...draft, username: e.target.value });
                  }}
                />
              </StatusRow>
              <StatusRow
                label={t("settings.network.password")}
                help={t("settings.network.passwordHelp")}>
                <div className="flex items-start gap-2">
                  <Input
                    size="sm"
                    type="password"
                    className="w-[420px]"
                    aria-label={t("settings.network.password")}
                    autoComplete="new-password"
                    maxLength={MAX_PROXY_PASSWORD_BYTES}
                    placeholder={
                      network.proxy_password
                        ? t("settings.network.passwordSaved")
                        : t("settings.network.passwordUnset")
                    }
                    value={draft.password}
                    error={shown("password")}
                    data-testid="network-password"
                    onChange={(e) => {
                      setDraft({ ...draft, password: e.target.value });
                    }}
                  />
                  {network.proxy_password && (
                    <Button
                      size="sm"
                      variant="ghost"
                      data-testid="network-password-clear"
                      onClick={clearPassword}>
                      {t("settings.network.passwordClear")}
                    </Button>
                  )}
                </div>
              </StatusRow>
              <StatusRow
                label={t("settings.network.noProxy")}
                help={t("settings.network.noProxyHelp")}>
                <Input
                  size="sm"
                  mono
                  className="w-[420px]"
                  aria-label={t("settings.network.noProxy")}
                  placeholder="localhost, corp.example.com, 10.0.0.0/8"
                  autoComplete="off"
                  spellCheck={false}
                  maxLength={MAX_NO_PROXY_BYTES}
                  value={draft.noProxy}
                  error={shown("no_proxy")}
                  data-testid="network-no-proxy"
                  onChange={(e) => {
                    setDraft({ ...draft, noProxy: e.target.value });
                  }}
                />
              </StatusRow>
              <div className="flex items-center justify-end gap-2 py-3">
                {proxy.mode !== "manual" && (
                  <span className="mr-auto text-[12px] text-fg-muted" data-testid="network-pending">
                    {t("settings.network.manualPending")}
                  </span>
                )}
                <Button size="sm" variant="ghost" disabled={!changed} onClick={revert}>
                  {t("settings.network.revert")}
                </Button>
                <Button
                  size="sm"
                  variant="primary"
                  disabled={!changed}
                  data-testid="network-save"
                  onClick={save}>
                  {t("settings.network.save")}
                </Button>
              </div>
            </>
          )}
        </SettingsRows>
      </SettingsSection>
    </SettingsPane>
  );
}
