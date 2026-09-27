// Settings navigation: the group ids and their mono keys. The labels live in the i18n dictionary
// (`settings.group.<id>`). Every pane reads the core: 语音模型 (speech) holds the recognition
// providers, the local models and the recognition options; AI 模型 (ai) the LLM providers behind the
// clean-up and voice edit; 场景 the scenes and the context switches (docs/dictation.md §18).
export const SETTINGS_GROUP_IDS = [
  "general",
  "hotkey",
  "speech",
  "ai",
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
