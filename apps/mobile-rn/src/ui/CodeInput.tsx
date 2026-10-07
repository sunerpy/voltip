// The six-digit pairing code: one real input under six drawn cells (`_ _ _   _ _ _`), the numeric
// keyboard, and `onComplete` once when the sixth digit lands.
import { useRef } from "react";
import { Pressable, TextInput, View } from "react-native";
import { Text } from "react-native-paper";

import { useT } from "../backend/i18n";
import { styles as kit, useAppTheme } from "./kit";

export const CODE_LENGTH = 6;

export function CodeInput({
  value,
  onChange,
  onComplete,
  disabled = false,
  error,
  autoFocus = false,
}: {
  value: string;
  onChange: (digits: string) => void;
  onComplete?: (digits: string) => void;
  disabled?: boolean;
  error?: string | undefined;
  autoFocus?: boolean;
}) {
  const theme = useAppTheme();
  const t = useT();
  const input = useRef<TextInput>(null);
  const digits = value.replace(/\D/g, "").slice(0, CODE_LENGTH);
  return (
    <View style={{ gap: 8 }}>
      <Pressable
        onPress={() => {
          input.current?.focus();
        }}
        accessibilityRole="none"
        style={{ flexDirection: "row", justifyContent: "center", gap: 8 }}>
        {Array.from({ length: CODE_LENGTH }, (_, i) => {
          const active = i === digits.length && !disabled;
          return (
            <View key={i} style={{ flexDirection: "row" }}>
              {i === 3 && <View style={{ width: 12 }} />}
              <View
                style={{
                  width: 44,
                  height: 56,
                  borderRadius: 12,
                  borderWidth: active ? 2 : 1,
                  borderColor:
                    error !== undefined
                      ? theme.colors.error
                      : active
                        ? theme.colors.primary
                        : theme.colors.outline,
                  alignItems: "center",
                  justifyContent: "center",
                  backgroundColor: theme.colors.surface,
                }}>
                <Text
                  variant="headlineSmall"
                  style={[
                    kit.mono,
                    {
                      color: digits[i] === undefined ? theme.voltip.subtle : theme.colors.onSurface,
                    },
                  ]}>
                  {digits[i] ?? "_"}
                </Text>
              </View>
            </View>
          );
        })}
      </Pressable>
      <TextInput
        ref={input}
        testID="pair-code-input"
        accessibilityLabel={t("ui.a11y.codeInput")}
        value={digits}
        editable={!disabled}
        autoFocus={autoFocus}
        keyboardType="number-pad"
        autoComplete="one-time-code"
        textContentType="oneTimeCode"
        maxLength={CODE_LENGTH}
        caretHidden
        onChangeText={(text) => {
          const next = text.replace(/\D/g, "").slice(0, CODE_LENGTH);
          onChange(next);
          if (next.length === CODE_LENGTH && next !== digits) onComplete?.(next);
        }}
        style={{ position: "absolute", width: 1, height: 1, opacity: 0 }}
      />
      {error !== undefined && (
        <Text
          variant="bodySmall"
          accessibilityRole="alert"
          style={{ color: theme.colors.error, textAlign: "center" }}>
          {error}
        </Text>
      )}
    </View>
  );
}
