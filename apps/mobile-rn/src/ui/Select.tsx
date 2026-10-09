// Choosing one of a few values (docs/mobile-rn.md §5): a field, or a settings row, that opens
// Material 3's modal bottom sheet with the choices, the current one checked (user request
// 2026-10-09: 下拉框都应优化为底部弹出的抽屉). The sheet slides up over a scrim; a choice, a tap on
// the scrim, a swipe down on its handle or Android's back closes it, and back closes nothing else.
// Never Android's old dialog of radio buttons.
import { useEffect, useMemo, useState } from "react";
import {
  Animated,
  BackHandler,
  Easing,
  Modal,
  PanResponder,
  Pressable,
  ScrollView,
  StyleSheet,
  View,
  useWindowDimensions,
} from "react-native";
import { Icon, List, Text, TouchableRipple } from "react-native-paper";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { useT } from "../backend/i18n";
import { useAppTheme } from "./kit";

export interface SelectOption<V extends string> {
  value: V;
  label: string;
  /** A second line under the label. */
  description?: string;
  disabled?: boolean;
}

interface SelectProps<V extends string> {
  label: string;
  value: V;
  options: readonly SelectOption<V>[];
  onChange: (value: V) => void;
  disabled?: boolean;
  testID?: string;
}

/** How far a swipe down on the handle takes the sheet away, or how fast. */
const DISMISS_DISTANCE = 80;
const DISMISS_VELOCITY = 0.8;
const OPEN_MS = 240;
const CLOSE_MS = 180;

/** While the sheet is open, Android's back closes it and nothing else. On a phone the modal's
 *  `onRequestClose` takes the press; this listener is for the cases where the press reaches the
 *  app's handlers first (and for the tests, where there is no modal window). */
function useBackCloses(open: boolean, close: () => void) {
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

/** The option `value` names, or the first one that can be chosen (as a native select shows it). */
function shown<V extends string>(
  options: readonly SelectOption<V>[],
  value: V,
): SelectOption<V> | undefined {
  return options.find((o) => o.value === value) ?? options.find((o) => o.disabled !== true);
}

/** The open state of a picker and what choosing does: the sheet closes, and a different value (or
 *  the first option standing in for a value no option has) goes to `onChange`. */
function usePicker<V extends string>({ value, options, onChange }: SelectProps<V>) {
  const [open, setOpen] = useState(false);
  const close = useMemo(() => () => setOpen(false), []);
  useBackCloses(open, close);
  const current = shown(options, value);
  const choose = (next: V) => {
    setOpen(false);
    if (next !== value || current?.value !== value) onChange(next);
  };
  return { open, setOpen, close, current, choose };
}

/** One choice in the sheet: its label (and description), a check at the end when it is the
 *  current one. */
function OptionRow<V extends string>({
  option,
  selected,
  choose,
  testID,
}: {
  option: SelectOption<V>;
  selected: boolean;
  choose: (v: V) => void;
  testID?: string;
}) {
  const theme = useAppTheme();
  const disabled = option.disabled === true;
  return (
    <TouchableRipple
      testID={testID === undefined ? undefined : `${testID}-option-${option.value}`}
      disabled={disabled}
      onPress={() => {
        choose(option.value);
      }}
      accessibilityRole="radio"
      accessibilityLabel={option.label}
      accessibilityHint={option.description}
      accessibilityState={{ checked: selected, selected, disabled }}
      style={[styles.option, { opacity: disabled ? 0.38 : 1 }]}>
      <View style={styles.optionInner}>
        <View style={{ flex: 1, gap: 2 }}>
          <Text
            variant="bodyLarge"
            style={{
              color: selected ? theme.colors.primary : theme.colors.onSurface,
              fontWeight: selected ? "600" : "400",
            }}>
            {option.label}
          </Text>
          {option.description !== undefined && (
            <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
              {option.description}
            </Text>
          )}
        </View>
        {selected && <Icon source="check" size={22} color={theme.colors.primary} />}
      </View>
    </TouchableRipple>
  );
}

/** Material 3's modal bottom sheet with the options of a picker titled `title`: it slides up from
 *  the bottom over a scrim and down again when it closes. */
export function OptionSheet<V extends string>({
  open,
  title,
  options,
  value,
  choose,
  close,
  testID,
}: {
  open: boolean;
  title: string;
  options: readonly SelectOption<V>[];
  value: V | undefined;
  choose: (v: V) => void;
  close: () => void;
  testID?: string;
}) {
  const theme = useAppTheme();
  const t = useT();
  const insets = useSafeAreaInsets();
  const { height } = useWindowDimensions();
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
            backgroundColor: theme.colors.elevation.level1,
            paddingBottom: insets.bottom + 12,
            maxHeight: height * 0.8,
            transform: [{ translateY }],
          },
        ]}>
        <View {...pan.panHandlers} style={styles.head}>
          <View style={[styles.handle, { backgroundColor: theme.colors.onSurfaceVariant }]} />
          <Text variant="titleMedium" accessibilityRole="header" style={styles.title}>
            {title}
          </Text>
        </View>
        <ScrollView bounces={false} contentContainerStyle={{ paddingBottom: 4 }}>
          {options.map((o) => (
            <OptionRow
              key={o.value}
              option={o}
              selected={o.value === value}
              choose={choose}
              {...(testID === undefined ? {} : { testID })}
            />
          ))}
        </ScrollView>
      </Animated.View>
    </Modal>
  );
}

/** A form field: the label over the chosen value and a chevron; the choices open in a sheet. */
export function SelectField<V extends string>(props: SelectProps<V>) {
  const { label, options, disabled, testID } = props;
  const theme = useAppTheme();
  const { open, setOpen, close, current, choose } = usePicker(props);
  return (
    <>
      <TouchableRipple
        testID={testID}
        disabled={disabled}
        onPress={() => {
          setOpen(true);
        }}
        accessibilityRole="button"
        accessibilityLabel={label}
        accessibilityHint={current?.label}
        accessibilityState={{ expanded: open, disabled: disabled === true }}
        style={{
          borderWidth: 1,
          borderColor: open ? theme.colors.primary : theme.colors.outline,
          borderRadius: 12,
          // The ripple and Android's focus highlight follow the corners, not the bounding box.
          overflow: "hidden",
          paddingHorizontal: 16,
          paddingVertical: 8,
          minHeight: 56,
          justifyContent: "center",
          opacity: disabled === true ? 0.5 : 1,
        }}>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
          <View style={{ flex: 1 }}>
            <Text
              variant="bodySmall"
              style={{ color: open ? theme.colors.primary : theme.colors.onSurfaceVariant }}>
              {label}
            </Text>
            <Text variant="bodyLarge" numberOfLines={1}>
              {current?.label ?? ""}
            </Text>
          </View>
          <Icon source="chevron-down" size={24} color={theme.colors.onSurfaceVariant} />
        </View>
      </TouchableRipple>
      <OptionSheet
        open={open && disabled !== true}
        title={label}
        options={options}
        value={current?.value}
        choose={choose}
        close={close}
        {...(testID === undefined ? {} : { testID })}
      />
    </>
  );
}

/** A settings row: the title, the chosen value under it; the choices open in a sheet. */
export function SelectRow<V extends string>(props: SelectProps<V> & { icon?: string }) {
  const { label, options, disabled, testID, icon } = props;
  const { open, setOpen, close, current, choose } = usePicker(props);
  return (
    <>
      <List.Item
        testID={testID}
        title={label}
        description={current?.label}
        disabled={disabled}
        onPress={() => {
          setOpen(true);
        }}
        accessibilityRole="button"
        accessibilityState={{ expanded: open, disabled: disabled === true }}
        left={icon === undefined ? undefined : (p) => <List.Icon {...p} icon={icon} />}
        right={(p) => <List.Icon {...p} icon="chevron-down" />}
      />
      <OptionSheet
        open={open && disabled !== true}
        title={label}
        options={options}
        value={current?.value}
        choose={choose}
        close={close}
        {...(testID === undefined ? {} : { testID })}
      />
    </>
  );
}

const styles = StyleSheet.create({
  sheet: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
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
  option: { minHeight: 56, justifyContent: "center", paddingHorizontal: 24, paddingVertical: 10 },
  optionInner: { flexDirection: "row", alignItems: "center", gap: 16 },
});
