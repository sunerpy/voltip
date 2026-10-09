// jest-expo on the build host: the native modules the app's screens reach are replaced here by
// what a test can observe. The app's own module (`VoltipNative`) is never loaded by the screens:
// tests render <App> on `@voltip/shared/mock`'s in-memory backend (src/test/render.tsx).
import "@testing-library/react-native/matchers";
import { configure } from "@testing-library/react-native";

// No test here measures how fast a page reacts: the `findBy…` / `waitFor` timeout only has to
// outlast a busy machine, where rendering a page can take longer than the default second
// (AGENTS.md "Tests on CI's machines").
configure({ asyncUtilTimeout: 10_000 });

jest.mock("expo-navigation-bar", () => ({ NavigationBar: () => null, setStyle: jest.fn() }));
jest.mock("expo-status-bar", () => ({ StatusBar: () => null }));
jest.mock("expo-localization", () => ({ getLocales: () => [{ languageTag: "zh-CN" }] }));
jest.mock("expo-haptics", () => ({
  impactAsync: jest.fn(() => Promise.resolve()),
  selectionAsync: jest.fn(() => Promise.resolve()),
  notificationAsync: jest.fn(() => Promise.resolve()),
  ImpactFeedbackStyle: { Medium: "medium" },
  NotificationFeedbackType: { Warning: "warning" },
}));
jest.mock("expo-camera", () => ({
  CameraView: () => null,
  useCameraPermissions: () => [{ granted: true, canAskAgain: true }, jest.fn()],
}));
jest.mock("expo-image-picker", () => ({
  launchImageLibraryAsync: jest.fn(() => Promise.resolve({ canceled: true, assets: null })),
}));
jest.mock("expo-file-system", () => ({ File: jest.fn() }));
