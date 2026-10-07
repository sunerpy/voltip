// Choosing one of a few values (docs/mobile-rn.md §5): Material 3's exposed dropdown menu, a list
// that opens under its field (above it near the bottom of the screen), never Android's old dialog of
// radio buttons. Paper's Menu does the placement and closes on a tap outside and on Android's back.
import { useEffect, useState } from "react";
import { BackHandler, View } from "react-native";
import { Icon, List, Menu, Text, TouchableRipple } from "react-native-paper";

import { useAppTheme, widthOf } from "./kit";

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

/** While the menu is open, Android's back closes it and nothing else: Paper's own listener closes
 *  the menu but lets the press through, and the page under it would go back too. */
function useBackCloses(open: boolean, setOpen: (open: boolean) => void) {
  useEffect(() => {
    if (!open) return;
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      setOpen(false);
      return true;
    });
    return () => {
      subscription.remove();
    };
  }, [open, setOpen]);
}

/** The option `value` names, or the first one that can be chosen (as a native select shows it). */
function shown<V extends string>(
  options: readonly SelectOption<V>[],
  value: V,
): SelectOption<V> | undefined {
  return options.find((o) => o.value === value) ?? options.find((o) => o.disabled !== true);
}

function Options<V extends string>({
  options,
  value,
  choose,
  testID,
}: {
  options: readonly SelectOption<V>[];
  value: V;
  choose: (v: V) => void;
  testID?: string;
}) {
  return options.map((o) => (
    <Menu.Item
      key={o.value}
      testID={testID === undefined ? undefined : `${testID}-option-${o.value}`}
      title={o.label}
      leadingIcon={o.value === value ? "check" : undefined}
      disabled={o.disabled}
      accessibilityState={{ selected: o.value === value, disabled: o.disabled === true }}
      onPress={() => {
        choose(o.value);
      }}
      style={{ maxWidth: 420 }}
    />
  ));
}

/** A form field: the label over the chosen value, a chevron, the menu as wide as the field. */
export function SelectField<V extends string>({
  label,
  value,
  options,
  onChange,
  disabled,
  testID,
}: SelectProps<V>) {
  const theme = useAppTheme();
  const [open, setOpen] = useState(false);
  const [width, setWidth] = useState(0);
  useBackCloses(open, setOpen);
  const current = shown(options, value);
  const choose = (next: V) => {
    setOpen(false);
    // A value no option has shows the first option; choosing that one is a change.
    if (next !== value || current?.value !== value) onChange(next);
  };
  return (
    <Menu
      visible={open && disabled !== true}
      onDismiss={() => {
        setOpen(false);
      }}
      anchorPosition="bottom"
      contentStyle={{ minWidth: width }}
      anchor={
        <TouchableRipple
          testID={testID}
          disabled={disabled}
          onLayout={(e) => {
            setWidth(widthOf(e));
          }}
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
            <Icon
              source={open ? "menu-up" : "menu-down"}
              size={24}
              color={theme.colors.onSurfaceVariant}
            />
          </View>
        </TouchableRipple>
      }>
      <Options
        options={options}
        value={value}
        choose={choose}
        {...(testID === undefined ? {} : { testID })}
      />
    </Menu>
  );
}

/** A settings row: the title, the chosen value under it, the menu opening at the row. */
export function SelectRow<V extends string>({
  label,
  value,
  options,
  onChange,
  disabled,
  testID,
  icon,
}: SelectProps<V> & { icon?: string }) {
  const [open, setOpen] = useState(false);
  const [width, setWidth] = useState(0);
  useBackCloses(open, setOpen);
  const current = shown(options, value);
  const choose = (next: V) => {
    setOpen(false);
    if (next !== value || current?.value !== value) onChange(next);
  };
  return (
    <Menu
      visible={open && disabled !== true}
      onDismiss={() => {
        setOpen(false);
      }}
      anchorPosition="bottom"
      contentStyle={{ minWidth: Math.max(0, width - 32) }}
      anchor={
        <View
          onLayout={(e) => {
            setWidth(widthOf(e));
          }}>
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
            right={(p) => <List.Icon {...p} icon="menu-down" />}
          />
        </View>
      }>
      <Options
        options={options}
        value={value}
        choose={choose}
        {...(testID === undefined ? {} : { testID })}
      />
    </Menu>
  );
}
