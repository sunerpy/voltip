// The camera for the computer's pairing QR code: full screen, the system's camera preview (expo-camera
// with ML Kit's barcode reader), a frame to aim with. The first Voltip pairing link it sees goes to
// the core as it is (`pairing_join_ticket`); the camera does nothing else with what it sees.
import { CameraView, useCameraPermissions } from "expo-camera";
import { useEffect, useRef } from "react";
import { StyleSheet, View } from "react-native";
import { IconButton, Text } from "react-native-paper";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { useBackend } from "../backend/BackendProvider";
import { useT } from "../backend/i18n";
import { useRootNavigation } from "../routes";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { isPairingLink } from "./Pair";

export function Scanner() {
  const { backend } = useBackend();
  const navigation = useRootNavigation();
  const shell = useShell();
  const t = useT();
  const insets = useSafeAreaInsets();
  const [permission, ask] = useCameraPermissions();
  const done = useRef(false);

  useEffect(() => {
    if (permission !== null && !permission.granted && permission.canAskAgain) void ask();
  }, [permission, ask]);

  const close = () => {
    navigation.goBack();
  };
  return (
    <View style={{ flex: 1, backgroundColor: "#000" }}>
      {permission?.granted === true ? (
        <CameraView
          style={StyleSheet.absoluteFill}
          facing="back"
          barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
          onBarcodeScanned={({ data }) => {
            if (done.current || !isPairingLink(data)) return;
            done.current = true;
            void backend.invoke("pairing_join_ticket", { uri: data.trim() });
            navigation.goBack();
          }}
        />
      ) : (
        <View
          style={{ flex: 1, alignItems: "center", justifyContent: "center", gap: 16, padding: 24 }}>
          <Text variant="bodyLarge" style={{ color: "#fff", textAlign: "center" }}>
            {t("mobile.pair.noCamera")}
          </Text>
          <Button
            mode="contained"
            onPress={() => {
              shell.toast(t("mobile.pair.scanCancelled"), "danger");
              close();
            }}>
            {t("mobile.back")}
          </Button>
        </View>
      )}
      <View
        pointerEvents="none"
        style={[StyleSheet.absoluteFill, { alignItems: "center", justifyContent: "center" }]}>
        <View
          style={{
            width: 240,
            height: 240,
            borderRadius: 24,
            borderWidth: 3,
            borderColor: "rgba(255,255,255,0.9)",
          }}
        />
        <Text
          variant="titleMedium"
          style={{ color: "#fff", marginTop: 24, textAlign: "center", paddingHorizontal: 32 }}>
          {t("mobile.pair.aim")}
        </Text>
      </View>
      <IconButton
        icon="close"
        iconColor="#fff"
        containerColor="rgba(0,0,0,0.4)"
        size={28}
        accessibilityLabel={t("mobile.back")}
        style={{ position: "absolute", top: insets.top + 8, left: 8 }}
        onPress={close}
      />
    </View>
  );
}
