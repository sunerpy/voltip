// Material 3's full-screen dialog, the phone's editor for anything longer than a field or two (a
// preset, a dictionary entry, a rule, a scene): a close button, the title and the confirming
// action in the top bar, the form scrolling under it. Android's back closes it like the button. A
// modal is a window of its own: menus (SelectField) open in its own portal host, not under it.
import type { ReactNode } from "react";
import { KeyboardAvoidingView, Modal, ScrollView, View } from "react-native";
import { Appbar, Portal } from "react-native-paper";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { useT } from "../backend/i18n";
import { Button } from "./Button";
import { useAppTheme } from "./kit";

export function FullScreenDialog({
  visible,
  title,
  onClose,
  action,
  children,
  testID,
}: {
  visible: boolean;
  title: string;
  onClose: () => void;
  /** The confirming button at the end of the bar (保存). */
  action?: { label: string; onPress: () => void; disabled?: boolean };
  children: ReactNode;
  testID?: string;
}) {
  const theme = useAppTheme();
  const t = useT();
  const insets = useSafeAreaInsets();
  return (
    <Modal
      visible={visible}
      animationType="slide"
      onRequestClose={onClose}
      statusBarTranslucent
      navigationBarTranslucent>
      <Portal.Host>
        <View style={{ flex: 1, backgroundColor: theme.colors.background }} testID={testID}>
          <Appbar.Header
            mode="small"
            statusBarHeight={insets.top}
            style={{ backgroundColor: theme.colors.background }}>
            <Appbar.Action icon="close" accessibilityLabel={t("common.close")} onPress={onClose} />
            <Appbar.Content title={title} />
            {action !== undefined && (
              <Button
                mode="text"
                disabled={action.disabled}
                onPress={action.onPress}
                style={{ marginRight: 8 }}
                testID="dialog-save">
                {action.label}
              </Button>
            )}
          </Appbar.Header>
          <KeyboardAvoidingView style={{ flex: 1 }} behavior="padding">
            <ScrollView
              contentContainerStyle={{ padding: 16, gap: 16, paddingBottom: insets.bottom + 32 }}
              keyboardShouldPersistTaps="handled">
              {children}
            </ScrollView>
          </KeyboardAvoidingView>
        </View>
      </Portal.Host>
    </Modal>
  );
}
