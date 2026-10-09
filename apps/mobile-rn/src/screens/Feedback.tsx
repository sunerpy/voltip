// 反馈 (apps/mobile's Feedback, docs/feedback.md): a kind, the user's words, an optional contact and
// up to three screenshots or recordings from the system's photo picker, and below the exact
// diagnostics that go along, shown before anything is sent. The shell stages the files and posts
// the report; the page never learns where to. A build without an endpoint offers the repository's
// issue page instead.
import {
  FEEDBACK_CONTACT_MAX,
  FEEDBACK_DIAGNOSTIC_ORDER,
  FEEDBACK_KINDS,
  FEEDBACK_LIMITS,
  FEEDBACK_MAX_ATTACHMENTS,
  FEEDBACK_MESSAGE_MAX,
  type FeedbackAttachmentError,
  type FeedbackError,
  type FeedbackInfo,
  type FeedbackKind,
  type StagedAttachment,
  attachmentError,
  attachmentSize,
  attachmentType,
  diagnosticValue,
  feedbackError,
  precheckAttachment,
} from "@voltip/shared";
import { File } from "expo-file-system";
import * as ImagePicker from "expo-image-picker";
import { useEffect, useState } from "react";
import { View } from "react-native";
import { Icon, IconButton, SegmentedButtons, Text, TextInput } from "react-native-paper";

import { useBackend } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useRootNavigation } from "../routes";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { Hint, Lede, Mono, Page, RowDivider, Section, useAppTheme } from "../ui/kit";

/** A file the picker handed over, as the shared checks read one. */
interface Picked {
  uri: string;
  name: string;
  type: string;
  size: number;
}

export function Feedback() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const navigation = useRootNavigation();
  const { t, locale } = useI18n();
  const [kind, setKind] = useState<FeedbackKind>("bug");
  const [message, setMessage] = useState("");
  const [contact, setContact] = useState("");
  const [files, setFiles] = useState<StagedAttachment[]>([]);
  const [info, setInfo] = useState<FeedbackInfo | undefined>(undefined);
  const [sending, setSending] = useState(false);
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<FeedbackError | undefined>(undefined);
  const [refusal, setRefusal] = useState<
    { reason: FeedbackAttachmentError; name: string } | undefined
  >(undefined);

  useEffect(() => {
    let live = true;
    void backend.feedbackAttachmentsClear().catch(() => undefined);
    void backend
      .feedbackDiagnostics(locale)
      .then((answer) => {
        if (live) setInfo(answer);
      })
      .catch(() => {
        if (live) setError("server");
      });
    return () => {
      live = false;
      void backend.feedbackAttachmentsClear().catch(() => undefined);
    };
  }, [backend, locale]);

  const configured = info?.configured ?? true;
  const empty = message.trim().length === 0;
  const submit = async () => {
    if (empty || sending || adding || !configured) return;
    setSending(true);
    setError(undefined);
    setRefusal(undefined);
    const trimmed = contact.trim();
    try {
      await backend.feedbackSubmit({
        kind,
        message,
        contact: trimmed.length > 0 ? trimmed : null,
        locale,
        ...(files.length > 0 ? { attachments: files.map((f) => f.id) } : {}),
      });
    } catch (e: unknown) {
      const reason = feedbackError(e);
      setSending(false);
      setError(reason);
      // The report went out; only a file did not follow. Sending again would send it twice.
      if (reason === "attachments") {
        setMessage("");
        setContact("");
        setFiles([]);
      }
      return;
    }
    setSending(false);
    shell.toast(t("feedback.sent"));
    navigation.goBack();
  };

  /** Stage `picked` in order, stopping at the first refusal, which is worded under the list. */
  const attach = async (picked: readonly Picked[]) => {
    if (picked.length === 0 || adding || sending) return;
    setAdding(true);
    setRefusal(undefined);
    let staged = files;
    for (const file of picked) {
      const type = attachmentType(file);
      const early = precheckAttachment(type, file.size, staged);
      if (early !== undefined) {
        setRefusal({ reason: early, name: file.name });
        break;
      }
      try {
        // oxlint-disable-next-line no-await-in-loop -- one at a time: each file's limits depend on the ones before
        const bytes = new Uint8Array(await new File(file.uri).arrayBuffer());
        // oxlint-disable-next-line no-await-in-loop -- as above
        const entry = await backend.feedbackAttachmentAdd({ name: file.name, type, bytes });
        staged = [...staged, entry];
        setFiles(staged);
      } catch (e: unknown) {
        setRefusal({ reason: attachmentError(e), name: file.name });
        break;
      }
    }
    setAdding(false);
  };
  const pick = async () => {
    const result = await ImagePicker.launchImageLibraryAsync({
      mediaTypes: ["images", "videos"],
      allowsMultipleSelection: true,
      selectionLimit: Math.max(1, FEEDBACK_MAX_ATTACHMENTS - files.length),
      quality: 1,
    });
    if (result.canceled) return;
    await attach(
      result.assets.map((a, i) => ({
        uri: a.uri,
        name: a.fileName ?? `attachment-${String(i + 1)}`,
        type: a.mimeType ?? "",
        size: a.fileSize ?? 0,
      })),
    );
  };
  const detach = (id: string) => {
    setFiles((current) => current.filter((f) => f.id !== id));
    setRefusal(undefined);
    void backend.feedbackAttachmentRemove(id).catch(() => undefined);
  };

  return (
    <Page testID="phone-feedback">
      <Lede>{t("mobile.feedback.lede")}</Lede>
      <Section padded>
        <SegmentedButtons
          value={kind}
          onValueChange={(value) => {
            setKind(value);
          }}
          buttons={FEEDBACK_KINDS.map((k) => ({ value: k, label: t(`feedback.kind.${k}`) }))}
        />
        <View style={{ gap: 4 }}>
          <TextInput
            mode="outlined"
            label={t("feedback.messageLabel")}
            placeholder={t("feedback.messagePlaceholder")}
            value={message}
            maxLength={FEEDBACK_MESSAGE_MAX}
            multiline
            numberOfLines={6}
            onChangeText={setMessage}
            testID="feedback-message"
          />
          <Mono
            testID="feedback-count"
            style={{ alignSelf: "flex-end", color: theme.voltip.subtle }}>
            {t("feedback.count", { n: message.length, max: FEEDBACK_MESSAGE_MAX })}
          </Mono>
        </View>
        <TextInput
          mode="outlined"
          label={t("feedback.contactLabel")}
          placeholder={t("feedback.contactPlaceholder")}
          value={contact}
          maxLength={FEEDBACK_CONTACT_MAX}
          autoCapitalize="none"
          onChangeText={setContact}
        />
      </Section>
      {configured && (
        <Section
          title={t("feedback.attachLabel")}
          testID="feedback-attachments"
          right={
            <Button
              compact
              icon="plus"
              loading={adding}
              disabled={adding || sending || files.length >= FEEDBACK_MAX_ATTACHMENTS}
              onPress={() => void pick()}>
              {adding ? t("feedback.attachAdding") : t("feedback.attachAdd")}
            </Button>
          }
          footer={
            <View style={{ gap: 4 }}>
              {refusal !== undefined && (
                <Hint tone="danger">
                  {t(`feedback.attachError.${refusal.reason}`, {
                    ...FEEDBACK_LIMITS,
                    name: refusal.name,
                  })}
                </Hint>
              )}
              <Hint>{t("mobile.feedback.attachHelp", FEEDBACK_LIMITS)}</Hint>
            </View>
          }>
          {files.length === 0 ? (
            <Text
              variant="bodyMedium"
              style={{ padding: 16, color: theme.colors.onSurfaceVariant }}>
              {t("feedback.attachAdd")}
            </Text>
          ) : (
            files.map((file, i) => {
              const video = file.type.startsWith("video/");
              return (
                <View key={file.id} testID="feedback-attachment">
                  {i > 0 && <RowDivider />}
                  <View
                    style={{
                      flexDirection: "row",
                      alignItems: "center",
                      gap: 12,
                      paddingLeft: 16,
                      minHeight: 56,
                    }}>
                    <Icon
                      source={video ? "video-outline" : "image-outline"}
                      size={20}
                      color={theme.colors.onSurfaceVariant}
                    />
                    <Text variant="bodyMedium" numberOfLines={1} style={{ flex: 1 }}>
                      {file.name}
                    </Text>
                    <Mono>{attachmentSize(file.size)}</Mono>
                    <IconButton
                      icon="close"
                      accessibilityLabel={t("feedback.attachRemove", { name: file.name })}
                      disabled={sending || adding}
                      onPress={() => detach(file.id)}
                    />
                  </View>
                </View>
              );
            })
          )}
        </Section>
      )}
      {!configured && <Hint>{t("feedback.notConfigured")}</Hint>}
      {error !== undefined && (
        <Hint tone="danger">{t(`feedback.error.${error}`, { max: FEEDBACK_MESSAGE_MAX })}</Hint>
      )}
      {configured ? (
        <Button
          mode="contained"
          icon="send"
          loading={sending}
          disabled={empty || sending || adding || info === undefined}
          onPress={() => void submit()}
          testID="feedback-send">
          {sending ? t("feedback.sending") : t("feedback.send")}
        </Button>
      ) : (
        <Button
          mode="contained"
          icon="open-in-new"
          onPress={() => {
            backend.projectLinkOpen("feedback").catch((e: unknown) => {
              shell.toast(
                t("mobile.toast.error", { message: e instanceof Error ? e.message : String(e) }),
                "danger",
              );
            });
          }}>
          {t("feedback.openIssue")}
        </Button>
      )}
      <Section
        title={t("feedback.attached")}
        testID="feedback-attached"
        footer={t("feedback.attachedHelp")}>
        {info === undefined ? (
          <Text variant="bodyMedium" style={{ padding: 16, color: theme.colors.onSurfaceVariant }}>
            {t("feedback.loading")}
          </Text>
        ) : (
          <View style={{ paddingVertical: 8 }}>
            {FEEDBACK_DIAGNOSTIC_ORDER.map((key) => {
              const value = info.diagnostics[key];
              if (value === undefined) return null;
              return (
                <View
                  key={key}
                  style={{
                    flexDirection: "row",
                    gap: 16,
                    paddingHorizontal: 16,
                    paddingVertical: 4,
                  }}>
                  <Text
                    variant="bodySmall"
                    style={{ width: 96, color: theme.colors.onSurfaceVariant }}>
                    {t(`feedback.diag.${key}`)}
                  </Text>
                  <Mono numberOfLines={2} style={{ flex: 1, color: theme.colors.onSurface }}>
                    {diagnosticValue(key, value, t, locale)}
                  </Mono>
                </View>
              );
            })}
          </View>
        )}
      </Section>
    </Page>
  );
}
