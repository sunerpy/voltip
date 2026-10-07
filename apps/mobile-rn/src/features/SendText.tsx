// The phone as the computer's keyboard (docs/dictation.md §20.6): type or paste a text and send
// it, or send the clipboard as it is; the computer inserts it at its cursor. The list below keeps
// what was sent, newest first, with the computer's answer. apps/mobile's SendText on native views.
import {
  type DeviceView,
  MAX_PHONE_TEXT_CHARS,
  type PhoneTextSource,
  type SentText,
  type TFunction,
  coreMessageText,
  formatDateTime,
  sentTextFinal,
} from "@voltip/shared";
import { useState } from "react";
import { View } from "react-native";
import { Text, TextInput } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { Mono, RowDivider, Section, StateLine, type Tone, useAppTheme } from "../ui/kit";
import { SelectField } from "../ui/Select";

/** Characters as the core counts them (code points), not UTF-16 units. */
function chars(text: string): number {
  return Array.from(text).length;
}

/** The line under a sent text: where it is on the computer. */
export function sentTextLine(text: SentText, t: TFunction): string {
  switch (text.state.state) {
    case "sending":
      return t("mobile.send.state.sending");
    case "queued":
      return t("mobile.send.state.queued", { name: text.device_name });
    case "delivered":
      return t(text.state.pasted ? "mobile.send.state.pasted" : "mobile.send.state.clipboard", {
        name: text.device_name,
      });
    case "failed":
      return t(`mobile.send.state.failed.${text.state.code}`, { message: text.state.message });
  }
}

function tone(text: SentText): Tone {
  if (text.state.state === "failed") return "danger";
  return sentTextFinal(text.state) ? "ok" : "accent";
}

export function SendText({ desktops }: { desktops: readonly DeviceView[] }) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const { sent_texts: sent } = useUiState();
  const online = desktops.filter((d) => d.connection.state === "online");
  const [picked, setPicked] = useState<string | undefined>(undefined);
  const target = online.find((d) => d.device.public_key === picked) ?? online[0];
  const [draft, setDraft] = useState("");
  const count = chars(draft);
  const tooLong = count > MAX_PHONE_TEXT_CHARS;
  const failed = (e: unknown) => {
    shell.toast(
      t("mobile.toast.error", {
        message: coreMessageText(e instanceof Error ? e.message : String(e)),
      }),
      "danger",
    );
  };
  const send = (body: string, source: PhoneTextSource) =>
    target === undefined
      ? Promise.resolve()
      : backend.invoke("phone_text_send", { publicKey: target.device.public_key, body, source });
  const sendDraft = async () => {
    try {
      await send(draft, "typed");
      setDraft("");
    } catch (e) {
      failed(e);
    }
  };
  const sendClipboard = async () => {
    try {
      const text = await backend.phoneClipboardRead();
      if (text === null || text.trim().length === 0) {
        shell.toast(t("mobile.send.clipboardEmpty"));
        return;
      }
      await send(text, "clipboard");
    } catch (e) {
      failed(e);
    }
  };

  return (
    <Section title={t("mobile.send.title")} padded testID="send-text">
      <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
        {target === undefined ? t("mobile.send.noDesktop") : t("mobile.send.body")}
      </Text>
      {target !== undefined && (
        <>
          {online.length > 1 && (
            <SelectField
              label={t("mobile.send.target")}
              value={target.device.public_key}
              options={online.map((d) => ({ value: d.device.public_key, label: d.device.name }))}
              onChange={setPicked}
            />
          )}
          <View style={{ gap: 4 }}>
            <TextInput
              mode="outlined"
              testID="send-text-draft"
              label={t("mobile.send.draft")}
              placeholder={t("mobile.send.placeholder")}
              multiline
              numberOfLines={3}
              value={draft}
              onChangeText={setDraft}
              error={tooLong}
            />
            <Mono
              style={{
                alignSelf: "flex-end",
                color: tooLong ? theme.colors.error : theme.voltip.subtle,
              }}>
              {t("mobile.send.count", { n: count, max: MAX_PHONE_TEXT_CHARS })}
            </Mono>
          </View>
          <View style={{ flexDirection: "row", gap: 8 }}>
            <Button
              mode="outlined"
              icon="content-paste"
              onPress={() => {
                void sendClipboard();
              }}>
              {t("mobile.send.clipboard")}
            </Button>
            <Button
              mode="contained"
              icon="send"
              style={{ flex: 1 }}
              disabled={draft.trim().length === 0 || tooLong}
              onPress={() => {
                void sendDraft();
              }}>
              {t("mobile.send.send", { name: target.device.name })}
            </Button>
          </View>
        </>
      )}
      {sent.length > 0 && (
        <View>
          <View
            style={{ flexDirection: "row", alignItems: "center", justifyContent: "space-between" }}>
            <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
              {t("mobile.send.sent", { n: sent.length })}
            </Text>
            <Button
              compact
              onPress={() => {
                void backend.invoke("sent_texts_clear");
              }}>
              {t("mobile.send.clear")}
            </Button>
          </View>
          {sent.map((text, i) => (
            <View key={`${text.device}-${text.id}`} testID="sent-text" style={{ gap: 4 }}>
              {i > 0 && <RowDivider />}
              <View style={{ gap: 4, paddingVertical: 8 }}>
                <Text variant="bodyMedium" numberOfLines={2}>
                  {text.body}
                </Text>
                <Mono style={{ color: theme.voltip.subtle }}>
                  {formatDateTime(locale, text.sent_at, { timeStyle: "short" })} ·{" "}
                  {t(`mobile.send.source.${text.source}`)} · {text.device_name}
                </Mono>
                <StateLine tone={tone(text)} pulse={!sentTextFinal(text.state)} small>
                  {sentTextLine(text, t)}
                </StateLine>
              </View>
            </View>
          ))}
        </View>
      )}
    </Section>
  );
}
