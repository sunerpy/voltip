// The safety code both devices show after the handshake (docs/pairing.md): four words and the
// fingerprint; the same on both screens when the channel is honest.
import type { SafetyCode } from "@voltip/shared";
import { View } from "react-native";
import { Text } from "react-native-paper";

import { useT } from "../backend/i18n";
import { Mono, styles as kit, useAppTheme } from "./kit";

export function SafetyCodeView({ code }: { code: SafetyCode }) {
  const theme = useAppTheme();
  const t = useT();
  return (
    <View style={{ alignItems: "center", gap: 12 }}>
      <View
        style={{ flexDirection: "row", flexWrap: "wrap", justifyContent: "center", gap: 8 }}
        accessibilityLabel={t("ui.a11y.safetyCode")}>
        {code.words.map((word, i) => (
          <View
            key={`${word}-${i}`}
            style={{
              minWidth: 72,
              paddingHorizontal: 12,
              paddingVertical: 10,
              borderRadius: 12,
              backgroundColor: theme.colors.surfaceVariant,
            }}>
            <Text
              variant="titleMedium"
              style={[kit.mono, { textAlign: "center" }]}
              testID="safety-word">
              {word}
            </Text>
          </View>
        ))}
      </View>
      <Mono selectable>{code.fingerprint}</Mono>
    </View>
  );
}
