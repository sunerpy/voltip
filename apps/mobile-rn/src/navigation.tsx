// The phone's screens (docs/mobile-rn.md §5): a native stack whose first screen holds the three
// tabs (说话 / 记录 / 设置, a dock floating over the page, Penpot 「澄空 Glass · Voltip」 board 03),
// with every other page pushed on top of it, so a detail page has the whole screen and the
// system's own transition. Android's back pops the stack; 记录 and 设置 go back to 说话; 说话 says a
// second back leaves (Talk.tsx).
import {
  BottomTabBarHeightCallbackContext,
  type BottomTabBarProps,
  createBottomTabNavigator,
} from "@react-navigation/bottom-tabs";
import { CommonActions } from "@react-navigation/native";
import {
  type NativeStackHeaderProps,
  createNativeStackNavigator,
} from "@react-navigation/native-stack";
import type { MessageKey } from "@voltip/shared";
import { type ComponentType, useContext } from "react";
import { StyleSheet, View } from "react-native";
import { Appbar, Icon, Text, TouchableRipple } from "react-native-paper";

import { useT } from "./backend/i18n";
import { About } from "./screens/About";
import { Bubble } from "./screens/Bubble";
import { AiModels } from "./screens/AiModels";
import { Appearance } from "./screens/Appearance";
import { ComputerSettings } from "./screens/ComputerSettings";
import { Dictionary } from "./screens/Dictionary";
import { Entry } from "./screens/Entry";
import { Feedback } from "./screens/Feedback";
import { History } from "./screens/History";
import { HistorySettings } from "./screens/HistorySettings";
import { Network } from "./screens/Network";
import { MirrorEntry } from "./screens/MirrorEntry";
import { Pair } from "./screens/Pair";
import { Recording } from "./screens/Recording";
import { Rules } from "./screens/Rules";
import { Scanner } from "./screens/Scanner";
import { Scenes } from "./screens/Scenes";
import { Settings } from "./screens/Settings";
import { SpeechModels } from "./screens/SpeechModels";
import { Talk } from "./screens/Talk";
import { ThisDevice } from "./screens/ThisDevice";
import { Usage } from "./screens/Usage";
import { Verify } from "./screens/Verify";
import { type RootParams, type TabParams } from "./routes";
import { cardShadow, useAppTheme } from "./ui/kit";

const Stack = createNativeStackNavigator<RootParams>();
const Tab = createBottomTabNavigator<TabParams>();

/** The top app bars sit on the page's own colour: one surface from the status bar down. */
function useHeaderStyle() {
  const theme = useAppTheme();
  return { backgroundColor: theme.colors.background };
}

/** Material 3's small top app bar: back when there is somewhere to go back to, the title, and the
 *  screen's actions. */
function StackHeader({ navigation, options, back }: NativeStackHeaderProps) {
  const t = useT();
  const style = useHeaderStyle();
  const title = typeof options.title === "string" ? options.title : "";
  return (
    <Appbar.Header mode="small" statusBarHeight={undefined} style={style}>
      {back !== undefined && (
        <Appbar.BackAction
          testID="header-back"
          accessibilityLabel={t("mobile.back")}
          onPress={() => {
            navigation.goBack();
          }}
        />
      )}
      <Appbar.Content title={title} />
      {options.headerRight?.({ canGoBack: back !== undefined })}
    </Appbar.Header>
  );
}

/** The dock's height and its gap to the bottom inset and to a snackbar above it (board 03). */
export const DOCK_HEIGHT = 64;
export const DOCK_GAP = 8;
/** The dock keeps a phone's width on a wide window (a tablet, a phone on its side). */
export const DOCK_MAX_WIDTH = 480;

/** How far the dock's bottom edge sits above the screen's: clear of the system's inset, and of
 *  the edge itself where there is none. */
export function dockBottom(inset: number): number {
  return Math.max(inset, 12) + DOCK_GAP;
}

/** Where a snackbar goes so that it clears the dock. */
export function aboveDock(inset: number): number {
  return dockBottom(inset) + DOCK_HEIGHT + DOCK_GAP;
}

/** The tabs' dock (board 03): a pill floating over the page, 24 dp from the sides (and their
 *  insets) and clear of the system's bottom inset, at most [DOCK_MAX_WIDTH] wide and centred, the
 *  chosen tab on a pill of the accent's soft tone. It tells the pages how tall it is, so their ends
 *  scroll clear of it. */
function TabDock({ state, descriptors, navigation, insets }: BottomTabBarProps) {
  const theme = useAppTheme();
  const t = useT();
  const reportHeight = useContext(BottomTabBarHeightCallbackContext);
  return (
    <View
      pointerEvents="box-none"
      onLayout={(e) => {
        reportHeight?.(e.nativeEvent.layout.height);
      }}
      style={[
        styles.dockWrap,
        {
          paddingBottom: dockBottom(insets.bottom),
          paddingTop: DOCK_GAP,
          paddingLeft: 24 + insets.left,
          paddingRight: 24 + insets.right,
        },
      ]}>
      <View
        testID="tab-dock"
        accessibilityRole="tablist"
        accessibilityLabel={t("mobile.tab.label")}
        style={[
          styles.dock,
          { backgroundColor: theme.voltip.dock, borderColor: theme.voltip.dockEdge },
          theme.voltip.glass ? cardShadow(theme, 16, 40, 0.16) : styles.dockFlat,
        ]}>
        {state.routes.map((route, index) => {
          const focused = state.index === index;
          const options = descriptors[route.key]?.options;
          const label = typeof options?.title === "string" ? options.title : route.name;
          const color = focused ? theme.colors.primary : theme.voltip.subtle;
          return (
            <View key={route.key} style={styles.tabCell}>
              <TouchableRipple
                testID={`tab-${route.name.toLowerCase()}`}
                accessibilityRole="tab"
                accessibilityLabel={label}
                accessibilityState={{ selected: focused }}
                borderless
                onPress={() => {
                  const event = navigation.emit({
                    type: "tabPress",
                    target: route.key,
                    canPreventDefault: true,
                  });
                  if (focused || event.defaultPrevented) return;
                  navigation.dispatch({
                    ...CommonActions.navigate(route.name, route.params),
                    target: state.key,
                  });
                }}
                style={[styles.tab, focused && { backgroundColor: theme.colors.primaryContainer }]}>
                <View style={styles.tabInner}>
                  {options?.tabBarIcon?.({ focused, color, size: 22 })}
                  <Text style={[styles.tabLabel, { color }]} numberOfLines={1}>
                    {label}
                  </Text>
                </View>
              </TouchableRipple>
            </View>
          );
        })}
      </View>
    </View>
  );
}

function tabIcon(name: string, focusedName: string) {
  return ({ focused, color }: { focused: boolean; color: string }) => (
    <Icon source={focused ? focusedName : name} size={22} color={color} />
  );
}

/** The three tabs. Each page draws its own title ([TabTitle]), so the aurora runs behind it. */
function Tabs() {
  const t = useT();
  return (
    <Tab.Navigator
      backBehavior="firstRoute"
      tabBar={(props) => <TabDock {...props} />}
      screenOptions={{ headerShown: false }}>
      <Tab.Screen
        name="Talk"
        component={Talk}
        options={{
          title: t("mobile.tab.talk"),
          tabBarIcon: tabIcon("microphone-outline", "microphone"),
        }}
      />
      <Tab.Screen
        name="History"
        component={History}
        options={{ title: t("mobile.tab.history"), tabBarIcon: tabIcon("history", "history") }}
      />
      <Tab.Screen
        name="Settings"
        component={Settings}
        options={{ title: t("mobile.tab.settings"), tabBarIcon: tabIcon("cog-outline", "cog") }}
      />
    </Tab.Navigator>
  );
}

const styles = StyleSheet.create({
  dockWrap: { position: "absolute", left: 0, right: 0, bottom: 0 },
  dock: {
    width: "100%",
    maxWidth: DOCK_MAX_WIDTH,
    alignSelf: "center",
    height: DOCK_HEIGHT,
    borderRadius: DOCK_HEIGHT / 2,
    borderWidth: 1,
    flexDirection: "row",
    alignItems: "center",
  },
  dockFlat: { elevation: 3 },
  tabCell: { flex: 1, alignItems: "center" },
  tab: { width: 82, height: 52, borderRadius: 26, justifyContent: "center" },
  tabInner: { alignItems: "center", gap: 2 },
  tabLabel: { fontSize: 11, lineHeight: 15, fontWeight: "500" },
});

/** Every pushed page, with the title key of apps/mobile's header. */
const PAGES: {
  name: Exclude<keyof RootParams, "Tabs" | "Scanner">;
  title: MessageKey;
  component: ComponentType<object>;
}[] = [
  { name: "ThisDevice", title: "mobile.title.device", component: ThisDevice },
  { name: "Pair", title: "mobile.title.pair", component: Pair },
  { name: "Verify", title: "mobile.title.verify", component: Verify },
  { name: "Speech", title: "mobile.title.speech", component: SpeechModels },
  { name: "Ai", title: "mobile.title.ai", component: AiModels },
  { name: "Appearance", title: "mobile.title.appearance", component: Appearance },
  { name: "Recording", title: "mobile.title.recording", component: Recording },
  { name: "About", title: "mobile.title.about", component: About },
  { name: "Dictionary", title: "mobile.title.dictionary", component: Dictionary },
  { name: "Rules", title: "mobile.title.rules", component: Rules },
  { name: "Scenes", title: "mobile.title.scenes", component: Scenes },
  { name: "HistorySettings", title: "mobile.title.historySettings", component: HistorySettings },
  { name: "Network", title: "mobile.title.network", component: Network },
  { name: "Feedback", title: "mobile.title.feedback", component: Feedback },
  { name: "Usage", title: "mobile.title.usage", component: Usage },
  { name: "Bubble", title: "mobile.title.bubble", component: Bubble },
  { name: "ComputerSettings", title: "mobile.title.computerSettings", component: ComputerSettings },
  { name: "Entry", title: "mobile.title.entry", component: Entry },
  { name: "MirrorEntry", title: "mobile.title.mirrorEntry", component: MirrorEntry },
];

export function RootNavigator({ initial }: { initial: "Tabs" | "Verify" }) {
  const t = useT();
  return (
    <Stack.Navigator
      initialRouteName={initial}
      screenOptions={{ header: (props) => <StackHeader {...props} />, animation: "default" }}>
      <Stack.Screen name="Tabs" component={Tabs} options={{ headerShown: false }} />
      {PAGES.map((page) => (
        <Stack.Screen
          key={page.name}
          name={page.name}
          component={page.component}
          options={{ title: t(page.title) }}
        />
      ))}
      <Stack.Screen
        name="Scanner"
        component={Scanner}
        options={{ headerShown: false, animation: "fade", presentation: "fullScreenModal" }}
      />
    </Stack.Navigator>
  );
}
