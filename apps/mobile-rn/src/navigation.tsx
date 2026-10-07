// The phone's screens (docs/mobile-rn.md §5): a native stack whose first screen holds the three
// tabs (说话 / 记录 / 设置, Material 3's navigation bar), with every other page pushed on top of it,
// so a detail page has the whole screen and the system's own transition. Android's back pops the
// stack; 记录 and 设置 go back to 说话; 说话 says a second back leaves (Talk.tsx).
import { type BottomTabBarProps, createBottomTabNavigator } from "@react-navigation/bottom-tabs";
import { CommonActions } from "@react-navigation/native";
import {
  type NativeStackHeaderProps,
  createNativeStackNavigator,
} from "@react-navigation/native-stack";
import type { MessageKey } from "@voltip/shared";
import type { ComponentType, ReactNode } from "react";
import { View } from "react-native";
import { Appbar, BottomNavigation, Icon } from "react-native-paper";

import { useUiState } from "./backend/BackendProvider";
import { useT } from "./backend/i18n";
import { About } from "./screens/About";
import { AiModels } from "./screens/AiModels";
import { Appearance } from "./screens/Appearance";
import { ComputerSettings } from "./screens/ComputerSettings";
import { Dictionary } from "./screens/Dictionary";
import { Entry } from "./screens/Entry";
import { Feedback } from "./screens/Feedback";
import { History } from "./screens/History";
import { HistorySettings } from "./screens/HistorySettings";
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
import { Verify } from "./screens/Verify";
import { type RootParams, type TabParams, useRootNavigation } from "./routes";
import { useAppTheme } from "./ui/kit";
import { Logo } from "./ui/Logo";

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

/** The navigation bar under the tabs (Material 3's NavigationBar), on the cards' white with a
 *  hairline above it; the selected tab is the accent itself, without the pill behind its icon. */
function TabBar({ state, descriptors, navigation, insets }: BottomTabBarProps) {
  const theme = useAppTheme();
  return (
    <BottomNavigation.Bar
      navigationState={state}
      safeAreaInsets={insets}
      shifting={false}
      style={{
        backgroundColor: theme.colors.surface,
        borderTopWidth: 1,
        borderTopColor: theme.colors.outlineVariant,
      }}
      activeColor={theme.colors.primary}
      inactiveColor={theme.colors.onSurfaceVariant}
      activeIndicatorStyle={{ backgroundColor: "transparent" }}
      onTabPress={(press) => {
        const { route } = press;
        const event = navigation.emit({
          type: "tabPress",
          target: route.key,
          canPreventDefault: true,
        });
        if (event.defaultPrevented) {
          press.preventDefault();
          return;
        }
        navigation.dispatch({
          ...CommonActions.navigate(route.name, route.params),
          target: state.key,
        });
      }}
      renderIcon={({ route, focused, color }) =>
        descriptors[route.key]?.options.tabBarIcon?.({ focused, color, size: 24 }) ?? null
      }
      getLabelText={({ route }) => {
        const label = descriptors[route.key]?.options.title;
        return typeof label === "string" ? label : route.name;
      }}
      getTestID={({ route }) => `tab-${route.name.toLowerCase()}`}
    />
  );
}

function tabIcon(name: string, focusedName: string) {
  return ({ focused, color }: { focused: boolean; color: string }) => (
    <Icon source={focused ? focusedName : name} size={24} color={color} />
  );
}

/** The talk tab's title: the mark and the name until a computer is paired, then 已配对设备 with
 *  the way to this phone's own page. */
function TalkHeader({ right }: { right?: ReactNode }) {
  const t = useT();
  const style = useHeaderStyle();
  const { devices } = useUiState();
  return (
    <Appbar.Header mode="small" style={style}>
      {devices.length === 0 && (
        <View style={{ marginLeft: 12 }}>
          <Logo size={26} />
        </View>
      )}
      <Appbar.Content title={devices.length === 0 ? "Voltip" : t("mobile.title.devices")} />
      {right}
    </Appbar.Header>
  );
}

function Tabs() {
  const t = useT();
  const style = useHeaderStyle();
  const navigation = useRootNavigation();
  return (
    <Tab.Navigator
      backBehavior="firstRoute"
      tabBar={(props) => <TabBar {...props} />}
      screenOptions={{
        header: ({ options }) => (
          <Appbar.Header mode="small" style={style}>
            <Appbar.Content title={typeof options.title === "string" ? options.title : ""} />
          </Appbar.Header>
        ),
      }}>
      <Tab.Screen
        name="Talk"
        component={Talk}
        options={{
          title: t("mobile.tab.talk"),
          tabBarIcon: tabIcon("microphone-outline", "microphone"),
          header: () => (
            <TalkHeader
              right={
                <Appbar.Action
                  icon="cellphone"
                  testID="open-this-device"
                  accessibilityLabel={t("mobile.thisDevice")}
                  onPress={() => {
                    navigation.navigate("ThisDevice");
                  }}
                />
              }
            />
          ),
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
  { name: "Feedback", title: "mobile.title.feedback", component: Feedback },
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
