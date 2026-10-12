// Choosing one of a few values (docs/mobile-rn.md §5): a field, or a settings row, that opens
// Material 3's modal bottom sheet with the choices, the current one checked (user request
// 2026-10-09: 下拉框都应优化为底部弹出的抽屉). The sheet slides up over a scrim; a choice, a tap on
// the scrim, a swipe down on its handle or Android's back closes it, and back closes nothing else.
// Never Android's old dialog of radio buttons.
import { useMemo, useState } from "react";
import { ScrollView, StyleSheet, View } from "react-native";
import { Icon, List, Text, TouchableRipple } from "react-native-paper";

import { Chip, useAppTheme } from "./kit";
import { BottomSheet, useBackCloses } from "./Sheet";

export interface SelectOption<V extends string> {
  value: V;
  label: string;
  /** A second line under the label. */
  description?: string;
  disabled?: boolean;
  /** The heading of the group the option is in: consecutive options with the same one form a
   *  group. A value may then appear in two groups; the first one is the current one. */
  group?: string;
}

interface SelectProps<V extends string> {
  label: string;
  value: V;
  options: readonly SelectOption<V>[];
  onChange: (value: V) => void;
  disabled?: boolean;
  testID?: string;
  /** The sheet opens: for options that change as they are shown, such as a provider's models
   *  (docs/dictation.md §3.3). */
  onOpen?: () => void;
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
function usePicker<V extends string>({ value, options, onChange, onOpen }: SelectProps<V>) {
  const [open, setOpen] = useState(false);
  const close = useMemo(() => () => setOpen(false), []);
  useBackCloses(open, close);
  const current = shown(options, value);
  const choose = (next: V) => {
    setOpen(false);
    if (next !== value || current?.value !== value) onChange(next);
  };
  const openSheet = () => {
    setOpen(true);
    onOpen?.();
  };
  return { open, openSheet, close, current, choose };
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

/** Material 3's modal bottom sheet with the options of a picker titled `title`. */
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
  return (
    <BottomSheet
      open={open}
      close={close}
      title={title}
      {...(testID === undefined ? {} : { testID })}>
      <ScrollView bounces={false} contentContainerStyle={{ paddingBottom: 4 }}>
        {options.map((o, i) => {
          // A value in two groups is the current one, and has its test id, where it first appears.
          const first = options.findIndex((x) => x.value === o.value) === i;
          return (
            <View key={`${i}:${o.value}`}>
              {o.group !== undefined && o.group !== options[i - 1]?.group && (
                <List.Subheader accessibilityRole="header">{o.group}</List.Subheader>
              )}
              <OptionRow
                option={o}
                selected={first && o.value === value}
                choose={choose}
                {...(testID === undefined ? {} : { testID: first ? testID : `${testID}-again` })}
              />
            </View>
          );
        })}
      </ScrollView>
    </BottomSheet>
  );
}

/** A form field: the label over the chosen value and a chevron; the choices open in a sheet. */
export function SelectField<V extends string>(props: SelectProps<V>) {
  const { label, options, disabled, testID } = props;
  const theme = useAppTheme();
  const { open, openSheet, close, current, choose } = usePicker(props);
  return (
    <>
      <TouchableRipple
        testID={testID}
        disabled={disabled}
        onPress={openSheet}
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
            <Text variant="bodyLarge" numberOfLines={2}>
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

/** A chip on the page (board 03's 场景 · 不使用场景 ⌄): the label and the chosen value; the choices
 *  open in a sheet. */
export function SelectChip<V extends string>(props: SelectProps<V> & { icon?: string }) {
  const { label, options, disabled, testID, icon } = props;
  const { open, openSheet, close, current, choose } = usePicker(props);
  return (
    <>
      <Chip
        {...(icon === undefined ? {} : { icon })}
        label={`${label} · ${current?.label ?? ""}`}
        {...(disabled === undefined ? {} : { disabled })}
        {...(testID === undefined ? {} : { testID })}
        onPress={openSheet}>
        {`${label} · ${current?.label ?? ""}`}
      </Chip>
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
  const { open, openSheet, close, current, choose } = usePicker(props);
  return (
    <>
      <List.Item
        testID={testID}
        title={label}
        description={current?.label}
        disabled={disabled}
        onPress={openSheet}
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
  option: { minHeight: 56, justifyContent: "center", paddingHorizontal: 24, paddingVertical: 10 },
  optionInner: { flexDirection: "row", alignItems: "center", gap: 16 },
});
