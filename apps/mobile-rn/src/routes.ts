// The phone's routes and the hooks screens use to move between them, apart from the navigator
// itself (navigation.tsx), which imports every screen.
import {
  type NavigatorScreenParams,
  type RouteProp,
  useNavigation,
  useRoute,
} from "@react-navigation/native";
import type { NativeStackNavigationProp } from "@react-navigation/native-stack";

export type TabParams = {
  Talk: undefined;
  /** `desktop`: a computer's key, to show the phone's copy of its history (docs/dictation.md §20.8). */
  History: { desktop?: string } | undefined;
  Settings: undefined;
};

export type RootParams = {
  Tabs: NavigatorScreenParams<TabParams> | undefined;
  ThisDevice: undefined;
  Pair: undefined;
  Verify: undefined;
  Scanner: undefined;
  Speech: undefined;
  Ai: undefined;
  Appearance: undefined;
  Recording: undefined;
  About: undefined;
  Dictionary: undefined;
  Rules: undefined;
  Scenes: undefined;
  HistorySettings: undefined;
  Feedback: undefined;
  ComputerSettings: { desktop: string };
  Entry: { id: string };
  MirrorEntry: { desktop: string; id: string };
};

export type RootNavigation = NativeStackNavigationProp<RootParams>;

/** The stack's navigation from any screen (tabs included). */
export function useRootNavigation(): RootNavigation {
  return useNavigation<RootNavigation>();
}

export function useRootRoute<R extends keyof RootParams>(): RouteProp<RootParams, R> {
  return useRoute<RouteProp<RootParams, R>>();
}
