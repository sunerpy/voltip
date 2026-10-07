// 本机 (apps/mobile's ThisDevice): this phone's name (renamed here), where its keys live, its
// fingerprint, LAN discovery, and the ways to pair a computer and to the paired ones.
import { platformLabel, shortKey } from "@voltip/shared";
import { useState } from "react";
import { View } from "react-native";
import { Text, TextInput } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useRootNavigation } from "../routes";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import {
  FactRow,
  Lede,
  Mono,
  Page,
  RowDivider,
  Section,
  StateLine,
  SwitchRow,
  useAppTheme,
} from "../ui/kit";

const NAME_MAX = 64;

export function ThisDevice() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const { identity, secret_backend, settings } = useUiState();
  const shell = useShell();
  const navigation = useRootNavigation();
  const { t, locale } = useI18n();
  const [name, setName] = useState(identity?.name ?? "");
  const [editing, setEditing] = useState(false);
  const dirty = identity !== null && name.trim() !== identity.name;

  return (
    <Page testID="phone-device">
      <Section padded>
        {identity === null ? (
          <StateLine tone="idle" pulse>
            {t("mobile.device.generating")}
          </StateLine>
        ) : editing ? (
          <View style={{ gap: 12 }}>
            <TextInput
              mode="outlined"
              label={t("mobile.device.name")}
              value={name}
              maxLength={NAME_MAX}
              autoFocus
              onChangeText={setName}
              right={<TextInput.Affix text={`${Array.from(name).length} / ${NAME_MAX}`} />}
            />
            <View style={{ flexDirection: "row", gap: 8 }}>
              <Button
                mode="contained"
                disabled={!dirty || name.trim().length === 0}
                onPress={() => {
                  void backend.invoke("device_rename", { name: name.trim() });
                  setEditing(false);
                  shell.toast(t("mobile.device.renamed"));
                }}>
                {t("mobile.device.save")}
              </Button>
              <Button
                onPress={() => {
                  setName(identity.name);
                  setEditing(false);
                }}>
                {t("mobile.device.cancel")}
              </Button>
            </View>
          </View>
        ) : (
          <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
            <View style={{ flex: 1, gap: 2 }}>
              <Text variant="labelSmall" style={{ color: theme.voltip.subtle }}>
                {t("mobile.device.name")}
              </Text>
              <Text variant="headlineSmall" testID="device-name">
                {identity.name}
              </Text>
            </View>
            <Button
              icon="pencil-outline"
              onPress={() => {
                setName(identity.name);
                setEditing(true);
              }}>
              {t("mobile.device.rename")}
            </Button>
          </View>
        )}
      </Section>
      {identity !== null && (
        <Section>
          <FactRow
            label={t("mobile.device.platform")}
            value={platformLabel(identity.platform, locale)}
          />
          <RowDivider />
          <FactRow label={t("mobile.device.keystore")} value={secret_backend} mono />
          <RowDivider />
          <View style={{ paddingHorizontal: 16, paddingVertical: 12, gap: 4 }}>
            <Text variant="bodyMedium" style={{ color: theme.colors.onSurfaceVariant }}>
              {t("mobile.device.fingerprint")}
            </Text>
            <Mono variant="titleMedium" selectable style={{ color: theme.colors.onSurface }}>
              {identity.fingerprint}
            </Mono>
            <Mono selectable>
              {t("mobile.device.publicKey", { key: shortKey(identity.public_key) })}
            </Mono>
          </View>
        </Section>
      )}
      <Lede>{t("mobile.device.note")}</Lede>
      <Section testID="lan-discovery">
        <SwitchRow
          icon="lan"
          title={t("mobile.device.lan")}
          description={t("mobile.device.lanHelp")}
          value={settings.lan_discovery}
          onValueChange={(enabled) => {
            void backend.invoke("settings_set_lan_discovery", { enabled });
          }}
        />
      </Section>
      <Button
        mode="contained"
        icon="qrcode-scan"
        disabled={identity === null}
        onPress={() => {
          navigation.navigate("Pair");
        }}>
        {t("mobile.device.pair")}
      </Button>
      <Button
        mode="outlined"
        onPress={() => {
          navigation.navigate("Tabs", { screen: "Talk" });
        }}>
        {t("mobile.device.viewDevices")}
      </Button>
    </Page>
  );
}
