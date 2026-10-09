// The phone's building blocks on Material Design 3 (docs/mobile-rn.md §5): react-native-paper's
// components in Voltip's palette (theme/themes.ts), in the "简约中性" style: grouped lists of rows
// on white cards with a hairline, a quiet grey title above each group, 16 dp from the edges; the
// accent is kept for what can be pressed or is selected. Every touch target is at least 48 dp
// (Paper's list rows and buttons already are).
import {
  Children,
  useContext,
  type ComponentProps,
  type ReactNode,
  isValidElement,
  useEffect,
  useState,
} from "react";
import {
  Animated,
  KeyboardAvoidingView,
  type LayoutChangeEvent,
  Platform,
  ScrollView,
  type StyleProp,
  StyleSheet,
  type TextStyle,
  View,
  type ViewStyle,
} from "react-native";
import { BottomTabBarHeightContext } from "@react-navigation/bottom-tabs";
import {
  AnimatedFAB,
  Icon,
  List,
  Surface,
  Switch,
  Text,
  TouchableRipple,
  useTheme,
} from "react-native-paper";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import type { AppTheme } from "../theme/themes";

export const useAppTheme = () => useTheme<AppTheme>();

/** The corner of a card (Sections, tiles, notices). */
export const CARD_RADIUS = 14;

/** A screen's scrolling body: 16 dp from the edges, its blocks 16 dp apart, scrolled clear of the
 *  keyboard. */
export function Page({
  children,
  testID,
  onEndReached,
  gap = 16,
}: {
  children: ReactNode;
  testID?: string;
  /** Called when the end of the content scrolls into view (paged lists). */
  onEndReached?: () => void;
  gap?: number;
}) {
  const theme = useAppTheme();
  // A page pushed on the stack runs to the bottom of the screen (edge to edge): its end stays above
  // the navigation bar. Under the tabs the tab bar takes that inset.
  const inTabs = useContext(BottomTabBarHeightContext) !== undefined;
  const insets = useSafeAreaInsets();
  const bottom = inTabs ? 0 : insets.bottom;
  return (
    <KeyboardAvoidingView
      style={{ flex: 1 }}
      behavior={Platform.OS === "ios" ? "padding" : undefined}>
      <ScrollView
        testID={testID}
        style={{ flex: 1, backgroundColor: theme.colors.background }}
        contentContainerStyle={[styles.page, { gap, paddingBottom: 32 + bottom }]}
        keyboardShouldPersistTaps="handled"
        scrollEventThrottle={64}
        onScroll={
          onEndReached === undefined
            ? undefined
            : ({ nativeEvent: e }) => {
                if (e.layoutMeasurement.height + e.contentOffset.y >= e.contentSize.height - 240)
                  onEndReached();
              }
        }>
        {children}
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

/** The sentence under a screen's title, before its first block. */
export function Lede({ children }: { children: ReactNode }) {
  const theme = useAppTheme();
  return (
    <Text
      variant="bodyMedium"
      style={{ color: theme.colors.onSurfaceVariant, paddingHorizontal: 4 }}>
      {children}
    </Text>
  );
}

/** Small print under a block. */
export function Hint({ children, tone }: { children: ReactNode; tone?: "danger" | "muted" }) {
  const theme = useAppTheme();
  return (
    <Text
      variant="bodySmall"
      style={{
        color: tone === "danger" ? theme.colors.error : theme.colors.onSurfaceVariant,
        paddingHorizontal: 4,
      }}>
      {children}
    </Text>
  );
}

/** The title above a group (a [Section]'s, or a page's own block): small and grey, so the accent
 *  stays for what can be pressed. */
export function SectionTitle({
  children,
  style,
}: {
  children: ReactNode;
  style?: StyleProp<TextStyle>;
}) {
  const theme = useAppTheme();
  return (
    <Text
      variant="titleSmall"
      accessibilityRole="header"
      style={[{ color: theme.colors.onSurfaceVariant, fontSize: 13, lineHeight: 18 }, style]}>
      {children}
    </Text>
  );
}

/** A group of rows: its title, the rows on one card with a hairline, dividers between them, a note
 *  under it. */
export function Section({
  title,
  right,
  footer,
  children,
  testID,
  padded = false,
}: {
  title?: string;
  right?: ReactNode;
  footer?: ReactNode;
  children: ReactNode;
  testID?: string;
  /** Content that is not rows (a form, a card body): 16 dp inside the container. */
  padded?: boolean;
}) {
  const theme = useAppTheme();
  return (
    <View style={{ gap: 8 }} testID={testID}>
      {(title !== undefined || right !== undefined) && (
        <View style={styles.sectionHead}>
          {title !== undefined && <SectionTitle style={{ flex: 1 }}>{title}</SectionTitle>}
          {right}
        </View>
      )}
      <Surface
        mode="flat"
        elevation={0}
        style={[
          styles.group,
          { backgroundColor: theme.colors.surface, borderColor: theme.colors.outlineVariant },
          padded && styles.groupPadded,
        ]}>
        {children}
      </Surface>
      {footer !== undefined && (typeof footer === "string" ? <Hint>{footer}</Hint> : footer)}
    </View>
  );
}

/** The hairline between two rows of a [Section]. */
export function RowDivider() {
  const theme = useAppTheme();
  return (
    <View
      style={{
        height: StyleSheet.hairlineWidth,
        backgroundColor: theme.colors.outlineVariant,
        marginLeft: 16,
      }}
    />
  );
}

/** Rows with a divider between each two (arrays and conditional rows flattened first). */
export function Rows({ children }: { children: ReactNode }) {
  const shown = Children.toArray(children);
  return (
    <>
      {shown.map((child, i) => (
        <View key={isValidElement(child) && child.key !== null ? child.key : i}>
          {i > 0 && <RowDivider />}
          {child}
        </View>
      ))}
    </>
  );
}

type ListIcon = ComponentProps<typeof List.Icon>["icon"];

/** A row that opens a page: icon, title, a one-line description, a chevron. */
export function NavRow({
  icon,
  title,
  description,
  onPress,
  testID,
  disabled,
}: {
  icon?: ListIcon;
  title: string;
  description?: string;
  onPress: () => void;
  testID?: string;
  disabled?: boolean;
}) {
  return (
    <List.Item
      testID={testID}
      title={title}
      titleNumberOfLines={2}
      description={description}
      descriptionNumberOfLines={2}
      disabled={disabled}
      onPress={onPress}
      accessibilityRole="button"
      left={icon === undefined ? undefined : (p) => <List.Icon {...p} icon={icon} />}
      right={(p) => <List.Icon {...p} icon="chevron-right" />}
    />
  );
}

/** A row with a switch: the whole row toggles it. */
export function SwitchRow({
  icon,
  title,
  description,
  value,
  onValueChange,
  disabled,
  testID,
}: {
  icon?: ListIcon;
  title: string;
  description?: string;
  value: boolean;
  onValueChange: (next: boolean) => void;
  disabled?: boolean;
  testID?: string;
}) {
  return (
    <List.Item
      testID={testID}
      title={title}
      titleNumberOfLines={2}
      description={description}
      descriptionNumberOfLines={4}
      disabled={disabled}
      onPress={() => {
        onValueChange(!value);
      }}
      accessibilityRole="switch"
      accessibilityState={{ checked: value, disabled: disabled === true }}
      left={icon === undefined ? undefined : (p) => <List.Icon {...p} icon={icon} />}
      right={() => (
        <View style={styles.switchWrap} pointerEvents="none">
          <Switch
            value={value}
            disabled={disabled}
            importantForAccessibility="no-hide-descendants"
          />
        </View>
      )}
    />
  );
}

/** A read-only fact: its label left, the value right (and wrapping under it when long). */
export function FactRow({
  label,
  value,
  mono,
  selectable,
}: {
  label: string;
  value: ReactNode;
  mono?: boolean;
  selectable?: boolean;
}) {
  const theme = useAppTheme();
  return (
    <View style={styles.fact}>
      <Text variant="bodyMedium" style={{ color: theme.colors.onSurfaceVariant, flexShrink: 0 }}>
        {label}
      </Text>
      {typeof value === "string" || typeof value === "number" ? (
        <Text
          variant="bodyMedium"
          selectable={selectable}
          style={[styles.factValue, mono === true && styles.mono]}>
          {value}
        </Text>
      ) : (
        <View style={styles.factValueBox}>{value}</View>
      )}
    </View>
  );
}

export type Tone = "ok" | "accent" | "danger" | "idle" | "warning";

export function toneColor(theme: AppTheme, tone: Tone): string {
  switch (tone) {
    case "ok":
      return theme.voltip.ok;
    case "accent":
      return theme.voltip.accent;
    case "danger":
      return theme.voltip.danger;
    case "warning":
      return theme.voltip.warning;
    case "idle":
      return theme.voltip.subtle;
  }
}

/** A status dot; `pulse` breathes while something is under way. */
export function Lamp({
  tone,
  pulse = false,
  size = 8,
  testID,
}: {
  tone: Tone;
  pulse?: boolean;
  size?: number;
  testID?: string;
}) {
  const theme = useAppTheme();
  const [opacity] = useState(() => new Animated.Value(1));
  useEffect(() => {
    if (!pulse) {
      opacity.setValue(1);
      return;
    }
    const loop = Animated.loop(
      Animated.sequence([
        Animated.timing(opacity, { toValue: 0.35, duration: 700, useNativeDriver: true }),
        Animated.timing(opacity, { toValue: 1, duration: 700, useNativeDriver: true }),
      ]),
    );
    loop.start();
    return () => {
      loop.stop();
    };
  }, [pulse, opacity]);
  return (
    <Animated.View
      testID={testID}
      style={{
        width: size,
        height: size,
        borderRadius: size / 2,
        backgroundColor: toneColor(theme, tone),
        opacity,
      }}
    />
  );
}

/** A status line that may wrap: the lamp beside its first line. */
export function StateLine({
  tone,
  pulse,
  children,
  small = false,
  testID,
  numberOfLines,
}: {
  tone: Tone;
  pulse?: boolean;
  children: ReactNode;
  small?: boolean;
  testID?: string;
  numberOfLines?: number;
}) {
  const theme = useAppTheme();
  return (
    <View style={styles.stateLine} testID={testID}>
      <View style={{ paddingTop: small ? 5 : 7 }}>
        <Lamp
          tone={tone}
          {...(pulse === undefined ? {} : { pulse })}
          size={small ? 6 : 8}
          {...(testID === undefined ? {} : { testID: `${testID}-lamp` })}
        />
      </View>
      <Text
        variant={small ? "bodySmall" : "bodyMedium"}
        numberOfLines={numberOfLines}
        style={{ flex: 1, color: theme.colors.onSurface }}>
        {children}
      </Text>
    </View>
  );
}

/** Nothing to show yet, and why. */
export function EmptyState({
  icon,
  title,
  children,
}: {
  icon: string;
  title: string;
  children?: ReactNode;
}) {
  const theme = useAppTheme();
  return (
    <View style={styles.empty}>
      <Icon source={icon} size={32} color={theme.voltip.subtle} />
      <Text variant="titleMedium" style={{ textAlign: "center" }}>
        {title}
      </Text>
      {children !== undefined && (
        <Text
          variant="bodyMedium"
          style={{ color: theme.colors.onSurfaceVariant, textAlign: "center" }}>
          {children}
        </Text>
      )}
    </View>
  );
}

/** An inline notice in a soft shade of its tone, with an action at its end when there is one. */
export function Notice({
  tone = "accent",
  icon,
  children,
  action,
  testID,
}: {
  tone?: "accent" | "danger" | "warning" | "ok";
  icon?: string;
  children: ReactNode;
  action?: ReactNode;
  testID?: string;
}) {
  const theme = useAppTheme();
  const shades: Record<"accent" | "danger" | "warning" | "ok", readonly [string, string]> = {
    accent: [theme.colors.primaryContainer, theme.colors.onPrimaryContainer],
    danger: [theme.voltip.dangerSoft, theme.voltip.danger],
    warning: [theme.voltip.warningSoft, theme.colors.onSurface],
    ok: [theme.voltip.okSoft, theme.voltip.okText],
  };
  const shade = shades[tone];
  return (
    <View
      testID={testID}
      accessibilityRole={tone === "danger" ? "alert" : undefined}
      style={[styles.notice, { backgroundColor: shade[0] }]}>
      {icon !== undefined && <Icon source={icon} size={20} color={shade[1]} />}
      <View style={{ flex: 1, gap: 4 }}>
        {typeof children === "string" ? (
          <Text variant="bodyMedium" style={{ color: shade[1] }}>
            {children}
          </Text>
        ) : (
          children
        )}
        {action}
      </View>
    </View>
  );
}

/** A row of LEDs for a level (0…1) and its peak. */
export function Meter({
  level,
  peak,
  segments = 32,
  label,
}: {
  level: number;
  peak?: number;
  segments?: number;
  label: string;
}) {
  const theme = useAppTheme();
  const lit = Math.round(level * segments);
  const peakAt = peak === undefined ? -1 : Math.min(segments - 1, Math.round(peak * segments) - 1);
  return (
    <View
      style={styles.meter}
      accessibilityRole="progressbar"
      accessibilityLabel={label}
      accessibilityValue={{ min: 0, max: 100, now: Math.round(level * 100) }}>
      {Array.from({ length: segments }, (_, i) => (
        <View
          key={i}
          style={[
            styles.led,
            {
              backgroundColor:
                i === peakAt
                  ? theme.voltip.accent
                  : i < lit
                    ? theme.colors.onSurface
                    : theme.colors.outlineVariant,
            },
          ]}
        />
      ))}
    </View>
  );
}

/** Text in the monospaced face (readouts, fingerprints, codes). */
export function Mono({
  children,
  style,
  selectable,
  numberOfLines,
  testID,
  variant = "bodySmall",
}: {
  children: ReactNode;
  style?: StyleProp<TextStyle>;
  selectable?: boolean;
  numberOfLines?: number;
  testID?: string;
  variant?: ComponentProps<typeof Text>["variant"];
}) {
  const theme = useAppTheme();
  return (
    <Text
      variant={variant}
      selectable={selectable}
      numberOfLines={numberOfLines}
      testID={testID}
      style={[styles.mono, { color: theme.colors.onSurfaceVariant }, style]}>
      {children}
    </Text>
  );
}

/** A tappable block (a card row that is not a list row). */
export function Pressable({
  onPress,
  children,
  style,
  label,
  testID,
}: {
  onPress: () => void;
  children: ReactNode;
  style?: StyleProp<ViewStyle>;
  label?: string;
  testID?: string;
}) {
  return (
    <TouchableRipple
      onPress={onPress}
      style={style}
      accessibilityRole="button"
      accessibilityLabel={label}
      testID={testID}
      borderless={false}>
      <View>{children}</View>
    </TouchableRipple>
  );
}

/** The width a child laid out at (for menus as wide as their field). */
export function widthOf(e: LayoutChangeEvent): number {
  return e.nativeEvent.layout.width;
}

export const styles = StyleSheet.create({
  page: { padding: 16, paddingBottom: 32 },
  sectionHead: { flexDirection: "row", alignItems: "center", paddingHorizontal: 4, minHeight: 24 },
  group: { borderRadius: CARD_RADIUS, overflow: "hidden", borderWidth: 1 },
  groupPadded: { padding: 16, gap: 12 },
  switchWrap: { justifyContent: "center", paddingLeft: 8 },
  fact: {
    flexDirection: "row",
    alignItems: "flex-start",
    justifyContent: "space-between",
    gap: 16,
    paddingHorizontal: 16,
    paddingVertical: 12,
  },
  factValue: { flexShrink: 1, textAlign: "right" },
  factValueBox: { flexShrink: 1, alignItems: "flex-end" },
  stateLine: { flexDirection: "row", alignItems: "flex-start", gap: 8 },
  empty: { alignItems: "center", gap: 8, paddingHorizontal: 24, paddingVertical: 32 },
  notice: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
    borderRadius: CARD_RADIUS,
    padding: 16,
  },
  meter: { flexDirection: "row", gap: 2, height: 8 },
  led: { flex: 1, borderRadius: 1 },
  mono: { fontFamily: Platform.select({ android: "monospace", default: "Menlo" }) },
});

/** The page's main action, floating at its bottom end (Material's extended FAB), clear of the
 *  navigation bar. */
export function FloatingAction({
  icon,
  label,
  onPress,
  disabled,
  testID,
}: {
  icon: string;
  label: string;
  onPress: () => void;
  disabled?: boolean;
  testID?: string;
}) {
  const insets = useSafeAreaInsets();
  return (
    <AnimatedFAB
      icon={icon}
      label={label}
      extended
      disabled={disabled}
      onPress={onPress}
      accessibilityLabel={label}
      testID={testID}
      style={{ position: "absolute", right: 16, bottom: 16 + insets.bottom }}
    />
  );
}
