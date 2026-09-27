// Settings navigation: the group ids and their mono keys. The labels live in the i18n dictionary
// (`settings.group.<id>`); the general / hotkey / appearance / engine / scene / about panes read the
// core (引擎 is the full engines pane: cloud + local models; 场景 the scenes and the context switches,
// docs/dictation.md §18), only the privacy pane is still sample rows (`settings.brief.privacy`).
export const SETTINGS_GROUP_IDS = [
  "general",
  "hotkey",
  "engine",
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
