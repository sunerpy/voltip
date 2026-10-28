// Material 3's modal bottom sheet (docs/mobile-rn.md §5): it slides up from the bottom over a
// scrim and down again when it closes; a tap on the scrim, a swipe down on its handle or Android's
// back closes it, and back closes nothing else. The pickers' choices (`Select`) and a provider's
// settings (`features/engines`) open in one. The keyboard lifts it, so a field near its bottom
// stays in view. On a wide window it keeps 640 dp and stays clear of the system's side insets; on
// a phone on its side it leaves the status bar's height free above it.
import { type ReactNode, useEffect, useMemo, useState } from "react";
import {
  Animated,
  BackHandler,
  Easing,
  KeyboardAvoidingView,
  Modal,
  PanResponder,
  Pressable,
  StyleSheet,
  View,
  useWindowDimensions,
} from "react-native";
import { Text } from "react-native-paper";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { useT } from "../backend/i18n";
import type { AppTheme } from "../theme/themes";
import { useAppTheme } from "./kit";

/** How far a swipe down on the handle takes the sheet away, or how fast. */
const DISMISS_DISTANCE = 80;
const DISMISS_VELOCITY = 0.8;
const OPEN_MS = 240;
const CLOSE_MS = 180;
/** The least room above an open sheet, under the status bar. */
const SHEET_TOP_GAP = 8;

/** The sheet's own colour. A field on it takes it too: Paper's outlined field fills itself with
 *  the page's background by default, a grey box on the sheet (Device Farm 2026-10-11). */
export function sheetColor(theme: AppTheme): string {
  return theme.colors.elevation.level1;
}

/** While the sheet is open, Android's back closes it and nothing else. On a phone the modal's
 *  `onRequestClose` takes the press; this listener is for the cases where the press reaches the
 *  app's handlers first (and for the tests, where there is no modal window). A sheet opened over
 *  another one listens last, so it closes first. */
export function useBackCloses(open: boolean, close: () => void) {
  useEffect(() => {
    if (!open) return;
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      close();
      return true;
    });
    return () => {
      subscription.remove();
    };
  }, [open, close]);
}

export function BottomSheet({
  open,
  close,
  title,
  header,
  children,
  maxHeight = 0.8,
  testID,
}: {
  open: boolean;
  close: () => void;
  /** The title under the handle. */
  title?: string;
  /** Anything else under the handle, in place of a title (a provider's name and state). */
  header?: ReactNode;
  /** The sheet's content; it scrolls itself when it may not fit. */
  children: ReactNode;
  /** The tallest the sheet gets, as a share of the window. */
  maxHeight?: number;
  testID?: string;
}) {
  const theme = useAppTheme();
  const t = useT();
  const insets = useSafeAreaInsets();
  const { height } = useWindowDimensions();
  useBackCloses(open, close);
  // Mounted from the open until the closing slide has ended.
  const [mounted, setMounted] = useState(open);
  if (open && !mounted) setMounted(true);
  const [progress] = useState(() => new Animated.Value(0));
  const [drag] = useState(() => new Animated.Value(0));
  useEffect(() => {
    if (open) {
      drag.setValue(0);
      const slide = Animated.timing(progress, {
        toValue: 1,
        duration: OPEN_MS,
        easing: Easing.out(Easing.cubic),
        useNativeDriver: true,
      });
      slide.start();
      return () => {
        slide.stop();
      };
    }
    const slide = Animated.timing(progress, {
      toValue: 0,
      duration: CLOSE_MS,
      easing: Easing.in(Easing.cubic),
      useNativeDriver: true,
    });
    slide.start(({ finished }) => {
      if (finished) setMounted(false);
    });
    return () => {
      slide.stop();
    };
  }, [open, progress, drag]);
  const pan = useMemo(
    () =>
      PanResponder.create({
        onMoveShouldSetPanResponder: (_e, g) => g.dy > 4 && Math.abs(g.dy) > Math.abs(g.dx),
        onPanResponderMove: (_e, g) => {
          drag.setValue(Math.max(0, g.dy));
        },
        onPanResponderRelease: (_e, g) => {
          if (g.dy > DISMISS_DISTANCE || g.vy > DISMISS_VELOCITY) {
            close();
          } else {
            Animated.spring(drag, { toValue: 0, useNativeDriver: true, bounciness: 4 }).start();
          }
        },
        onPanResponderTerminate: () => {
          Animated.spring(drag, { toValue: 0, useNativeDriver: true }).start();
        },
      }),
    [drag, close],
  );
  if (!open && !mounted) return null;
  const translateY = Animated.add(
    progress.interpolate({ inputRange: [0, 1], outputRange: [height, 0] }),
    drag,
  );
  return (
    <Modal
      visible
      transparent
      animationType="none"
      statusBarTranslucent
      navigationBarTranslucent
      onRequestClose={close}>
      <KeyboardAvoidingView
        behavior="padding"
        style={[styles.frame, { paddingLeft: insets.left, paddingRight: insets.right }]}>
        <Animated.View
          style={[
            StyleSheet.absoluteFill,
            { backgroundColor: theme.colors.backdrop, opacity: progress },
          ]}>
          <Pressable
            style={StyleSheet.absoluteFill}
            onPress={close}
            accessibilityRole="button"
            accessibilityLabel={t("common.close")}
            testID={testID === undefined ? undefined : `${testID}-scrim`}
          />
        </Animated.View>
        <Animated.View
          accessibilityViewIsModal
          testID={testID === undefined ? undefined : `${testID}-sheet`}
          style={[
            styles.sheet,
            {
              backgroundColor: sheetColor(theme),
              paddingBottom: insets.bottom + 12,
              maxHeight: Math.min(height * maxHeight, height - insets.top - SHEET_TOP_GAP),
              transform: [{ translateY }],
            },
          ]}>
          <View
            {...pan.panHandlers}
            style={styles.head}
            testID={testID === undefined ? undefined : `${testID}-handle`}>
            <View style={[styles.handle, { backgroundColor: theme.colors.onSurfaceVariant }]} />
            {title !== undefined && (
              <Text variant="titleMedium" accessibilityRole="header" style={styles.title}>
                {title}
              </Text>
            )}
            {header}
          </View>
          {children}
        </Animated.View>
      </KeyboardAvoidingView>
    </Modal>
  );
}

const styles = StyleSheet.create({
  frame: { flex: 1, justifyContent: "flex-end" },
  sheet: {
    alignSelf: "center",
    width: "100%",
    maxWidth: 640,
    borderTopLeftRadius: 28,
    borderTopRightRadius: 28,
    overflow: "hidden",
  },
  head: { alignItems: "center", paddingTop: 12, paddingBottom: 8 },
  handle: { width: 32, height: 4, borderRadius: 2, opacity: 0.4 },
  title: { alignSelf: "stretch", paddingHorizontal: 24, paddingTop: 16, paddingBottom: 4 },
});
