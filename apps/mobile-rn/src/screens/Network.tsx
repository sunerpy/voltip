// 网络 (docs/dictation.md §24): the proxy the phone's own connections go through, the desktop's
// 设置 › 网络 on the phone — the current network's proxy (the default, handed to the shell from
// Android's network callback), none, or one set here. 跟随系统 and 不使用代理 apply at once; a
// manual proxy is checked as the core checks it and applies on 保存. Its password goes to the
// keystore and never comes back: the page only says whether one is saved.
import {
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
import { type ComponentProps, useEffect, useState } from "react";
import { View } from "react-native";
import { TextInput } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { FactRow, Hint, Lede, Page, Section } from "../ui/kit";
import { SelectRow } from "../ui/Select";

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
    noProxy: proxy.no_proxy,
  };
}

const MODE_LABEL = {
  system: "settings.network.modeSystem",
  off: "settings.network.modeOff",
  manual: "settings.network.modeManual",
} as const satisfies Record<ProxyMode, string>;

export function Network() {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const { settings, network } = useUiState();
  const proxy = settings.proxy;
  const [mode, setMode] = useState<ProxyMode>(proxy.mode);
  const [draft, setDraft] = useState<Draft>(() => draftOf(proxy));
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
    shell.toast(t("mobile.toast.error", { message: errorText(e) }), "danger");
  };
  const send = (next: ProxySettings, password: string | null) =>
    backend.invoke("settings_set_proxy", proxyArgs(next, password));
  const choose = (next: ProxyMode) => {
    setMode(next);
    setTried(false);
    if (next === proxy.mode || (next === "manual" && proxy.server === null)) return;
    send({ ...proxy, mode: next }, null).catch(fail);
  };

  const checked = checkProxySettings({
    mode: "manual",
    server: draft.server,
    username: draft.username,
    no_proxy: draft.noProxy,
  });
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
      shell.toast(t("settings.network.saved"));
    }, fail);
  };

  const help =
    mode === "system"
      ? t("mobile.network.systemHelp")
      : mode === "off"
        ? t("settings.network.offHelp")
        : t("settings.network.manualHelp");
  const field = (
    key: "server" | "username" | "noProxy",
    label: string,
    hint: string,
    problemOf: ProxyProblem["field"],
    extra: Partial<ComponentProps<typeof TextInput>>,
  ) => {
    const message = shown(problemOf);
    return (
      <View style={{ gap: 4 }}>
        <TextInput
          mode="outlined"
          label={label}
          value={draft[key]}
          autoCapitalize="none"
          autoCorrect={false}
          autoComplete="off"
          error={message !== undefined}
          onChangeText={(value) => {
            setDraft({ ...draft, [key]: value });
          }}
          {...extra}
        />
        {message === undefined ? <Hint>{hint}</Hint> : <Hint tone="danger">{message}</Hint>}
      </View>
    );
  };
  const passwordProblem = shown("password");

  return (
    <Page testID="phone-network">
      <Lede>{t("mobile.network.lede")}</Lede>
      <Section footer={help}>
        <SelectRow
          icon="web"
          label={t("settings.network.mode")}
          value={mode}
          options={PROXY_MODES.map((value) => ({ value, label: t(MODE_LABEL[value]) }))}
          onChange={choose}
          testID="network-mode"
        />
        {mode === "system" && (
          <FactRow
            label={t("settings.network.systemProxy")}
            value={network.system_proxy ?? t("settings.network.systemProxyNone")}
            mono={network.system_proxy !== undefined}
          />
        )}
      </Section>
      {mode === "manual" && (
        <Section
          padded
          testID="network-manual"
          {...(proxy.mode === "manual" ? {} : { footer: t("settings.network.manualPending") })}>
          <View style={{ gap: 12 }}>
            {field(
              "server",
              t("settings.network.server"),
              t("settings.network.serverHelp"),
              "server",
              {
                placeholder: "http://127.0.0.1:7890",
                keyboardType: "url",
                testID: "network-server",
              },
            )}
            {field(
              "username",
              t("settings.network.username"),
              t("settings.network.usernameHelp"),
              "username",
              {
                maxLength: MAX_PROXY_USERNAME_BYTES,
                testID: "network-username",
              },
            )}
            <View style={{ gap: 4 }}>
              <TextInput
                mode="outlined"
                label={t("settings.network.password")}
                value={draft.password}
                placeholder={
                  network.proxy_password
                    ? t("settings.network.passwordSaved")
                    : t("settings.network.passwordUnset")
                }
                secureTextEntry
                autoCapitalize="none"
                autoCorrect={false}
                autoComplete="off"
                maxLength={MAX_PROXY_PASSWORD_BYTES}
                error={passwordProblem !== undefined}
                testID="network-password"
                onChangeText={(password) => {
                  setDraft({ ...draft, password });
                }}
              />
              {passwordProblem === undefined ? (
                <Hint>{t("settings.network.passwordHelp")}</Hint>
              ) : (
                <Hint tone="danger">{passwordProblem}</Hint>
              )}
              {network.proxy_password && (
                <View style={{ flexDirection: "row" }}>
                  <Button
                    compact
                    icon="trash-can-outline"
                    testID="network-password-clear"
                    onPress={() => {
                      send(proxy, "").then(() => {
                        shell.toast(t("settings.network.passwordCleared"));
                      }, fail);
                    }}>
                    {t("settings.network.passwordClear")}
                  </Button>
                </View>
              )}
            </View>
            {field(
              "noProxy",
              t("settings.network.noProxy"),
              t("settings.network.noProxyHelp"),
              "no_proxy",
              {
                placeholder: "localhost, corp.example.com",
                maxLength: MAX_NO_PROXY_BYTES,
                testID: "network-no-proxy",
              },
            )}
            <View style={{ flexDirection: "row", gap: 8 }}>
              <Button mode="contained" disabled={!changed} onPress={save} testID="network-save">
                {t("settings.network.save")}
              </Button>
              <Button
                disabled={!changed}
                onPress={() => {
                  setDraft(draftOf(proxy));
                  setTried(false);
                  setMode(proxy.mode);
                }}>
                {t("settings.network.revert")}
              </Button>
            </View>
          </View>
        </Section>
      )}
    </Page>
  );
}
