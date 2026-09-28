// Settings navigation: the group ids and their mono keys. The labels live in the i18n dictionary
// (`settings.group.<id>`). Every pane reads the core; 场景 holds the scenes and the context switches
// (docs/dictation.md §18). 语音模型 and AI 模型 are pages of the main layout since 2026-09-28.
export const SETTINGS_GROUP_IDS = [
  "general",
  "hotkey",
  "scene",
  "privacy",
  "appearance",
  "about",
] as const;
export type SettingsGroupId = (typeof SETTINGS_GROUP_IDS)[number];

export interface SettingsGroup {
  id: SettingsGroupId;
  key: string;
}

export const settingsGroups: SettingsGroup[] = SETTINGS_GROUP_IDS.map((id) => ({ id, key: id }));
