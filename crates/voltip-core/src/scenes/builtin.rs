//! The built-in scenes (docs/dictation.md §18.7): seven categories the desktop's scene list always
//! holds, each a preset, an extra instruction, a term pack ([`crate::vocabulary::packs`]) and the
//! applications it applies to. The store appends a missing one, switched off, after the user's own
//! scenes; the user can switch it on, change its applications, preset and instruction, and restore
//! the defaults, but not delete or rename it. The four domains (法律, 医疗, 金融, 学术) list no
//! application: they apply once the user adds the software they use.
//!
//! The default applications are listed per platform, by the id the foreground probe reports there:
//! the Windows executable name, the macOS bundle id, the Linux X11 `WM_CLASS` class.

use serde::{Deserialize, Serialize};
use voltip_protocol::Platform;

use super::{SceneDraft, SceneMatch, SceneOverrides, normalize_app_id};
use crate::presets::{BuiltinPreset, PresetId};

/// One built-in scene category; on the wire and in `scenes.json` its snake-case name, which is also
/// the scene's stored `name` (the interface shows its own name for it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinScene {
    /// 编程开发.
    Coding,
    /// 办公写作.
    Office,
    /// 即时聊天.
    Chat,
    /// 法律.
    Legal,
    /// 医疗.
    Medical,
    /// 金融.
    Finance,
    /// 学术.
    Academic,
}

/// What every domain scene asks of the clean-up, then the domain's own sentence.
macro_rules! domain {
    ($rest:literal) => {
        concat!("严格校对，保持专业术语原样；数字、单位、日期写规范。", $rest)
    };
}

impl BuiltinScene {
    /// Every category, in the order the store appends them.
    pub const ALL: [Self; 7] = [Self::Coding, Self::Office, Self::Chat, Self::Legal, Self::Medical, Self::Finance, Self::Academic];

    /// The wire name (`coding`, …), the scene's stored `name`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Coding => "coding",
            Self::Office => "office",
            Self::Chat => "chat",
            Self::Legal => "legal",
            Self::Medical => "medical",
            Self::Finance => "finance",
            Self::Academic => "academic",
        }
    }

    /// The Chinese name (the interface names it in its own language).
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Coding => "编程开发",
            Self::Office => "办公写作",
            Self::Chat => "即时聊天",
            Self::Legal => "法律",
            Self::Medical => "医疗",
            Self::Finance => "金融",
            Self::Academic => "学术",
        }
    }

    /// The preset the category's takes refine with.
    pub const fn preset(self) -> BuiltinPreset {
        match self {
            Self::Office => BuiltinPreset::Formal,
            Self::Chat => BuiltinPreset::Chat,
            Self::Coding | Self::Legal | Self::Medical | Self::Finance | Self::Academic => BuiltinPreset::Proofread,
        }
    }

    /// The extra instruction for the clean-up; 即时聊天 has none (its preset says it all).
    pub const fn instruction(self) -> Option<&'static str> {
        match self {
            Self::Coding => Some("保留代码、命令、路径、英文标识符和 Markdown，不把技术词翻译成中文。"),
            Self::Office => Some("邮件和文档：句子完整，自然分段，不编造称呼和事实。"),
            Self::Chat => None,
            Self::Legal => Some(domain!("这是法律文本：法条、案号和当事人名称照原样保留。")),
            Self::Medical => Some(domain!("这是医疗文本：药名、剂量和检查项目照原样保留，不改动医学含义。")),
            Self::Finance => Some(domain!("这是金融文本：金额、百分比、证券代码和机构名称照原样保留。")),
            Self::Academic => Some(domain!("这是学术文本：引用、公式符号和专有名词照原样保留。")),
        }
    }

    /// The applications the category applies to on `platform`, as its probe reports them
    /// (normalised by [`normalize_app_id`] in [`Self::template`]); none for the domains and on a phone.
    pub fn default_apps(self, platform: Platform) -> &'static [&'static str] {
        match (self, platform) {
            (Self::Coding, Platform::Windows) => &[
                "code",
                "cursor",
                "kiro",
                "idea64",
                "pycharm64",
                "webstorm64",
                "goland64",
                "clion64",
                "rider64",
                "rustrover64",
                "windowsterminal",
                "powershell",
                "pwsh",
                "cmd",
                "wezterm-gui",
                "alacritty",
            ],
            (Self::Coding, Platform::Macos) => &[
                "com.microsoft.vscode",
                "com.todesktop.230313mzl4w4u92",
                "dev.kiro.desktop",
                "com.jetbrains.intellij",
                "com.jetbrains.intellij.ce",
                "com.jetbrains.pycharm",
                "com.jetbrains.pycharm.ce",
                "com.jetbrains.webstorm",
                "com.jetbrains.goland",
                "com.jetbrains.clion",
                "com.jetbrains.rustrover",
                "com.apple.terminal",
                "com.googlecode.iterm2",
                "com.mitchellh.ghostty",
                "dev.warp.warp-stable",
                "net.kovidgoyal.kitty",
                "org.alacritty",
                "com.github.wez.wezterm",
            ],
            (Self::Coding, Platform::Linux) => &[
                "code",
                "cursor",
                "kiro",
                "jetbrains-idea",
                "jetbrains-idea-ce",
                "jetbrains-pycharm",
                "jetbrains-pycharm-ce",
                "jetbrains-webstorm",
                "jetbrains-goland",
                "jetbrains-clion",
                "jetbrains-rustrover",
                "gnome-terminal-server",
                "konsole",
                "xfce4-terminal",
                "alacritty",
                "kitty",
                "org.wezfurlong.wezterm",
                "tilix",
                "xterm",
            ],
            (Self::Office, Platform::Windows) => &["outlook", "olk", "winword", "wps", "onenote", "notion", "obsidian", "typora"],
            (Self::Office, Platform::Macos) => &[
                "com.microsoft.outlook",
                "com.microsoft.word",
                "com.kingsoft.wpsoffice.mac",
                "com.apple.mail",
                "com.apple.iwork.pages",
                "notion.id",
                "md.obsidian",
                "abnerworks.typora",
            ],
            (Self::Office, Platform::Linux) => &["thunderbird", "libreoffice-writer", "wps", "evolution", "obsidian", "typora"],
            (Self::Chat, Platform::Windows) => {
                &["weixin", "wechat", "wxwork", "qq", "slack", "ms-teams", "teams", "dingtalk", "feishu", "lark", "telegram", "discord"]
            }
            (Self::Chat, Platform::Macos) => &[
                "com.tencent.xinwechat",
                "com.tencent.weworkmac",
                "com.tencent.qq",
                "com.tinyspeck.slackmacgap",
                "com.microsoft.teams2",
                "com.alibaba.dingtalkmac",
                "com.bytedance.macos.feishu",
                "com.electron.lark",
                "ru.keepcoder.telegram",
                "com.tdesktop.telegram",
                "com.hnc.discord",
                "com.apple.mobilesms",
            ],
            (Self::Chat, Platform::Linux) => &["wechat", "qq", "slack", "teams-for-linux", "dingtalk", "feishu", "lark", "telegramdesktop", "discord"],
            _ => &[],
        }
    }

    /// The category's defaults on `platform`: switched off, its applications, its preset and
    /// instruction, every other override following the global settings.
    pub fn template(self, platform: Platform) -> SceneDraft {
        SceneDraft {
            name: self.as_str().to_owned(),
            enabled: false,
            matching: SceneMatch { apps: self.default_apps(platform).iter().map(|app| normalize_app_id(app)).collect(), title_contains: Vec::new() },
            overrides: SceneOverrides {
                refine_preset: Some(PresetId::Builtin(self.preset())),
                prompt: self.instruction().map(str::to_owned),
                ..SceneOverrides::default()
            },
        }
    }

    /// The category a stored scene's `name` names.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == name)
    }
}

/// Whether `platform` keeps built-in scenes (the phone has no scenes at all).
pub const fn has_builtin_scenes(platform: Platform) -> bool {
    matches!(platform, Platform::Windows | Platform::Macos | Platform::Linux)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::{MAX_SCENE_APPS, MAX_SCENE_PROMPT_CHARS, validate_scene_draft_with};

    #[test]
    fn every_template_is_a_valid_scene_in_its_normalised_form_on_every_desktop() {
        for platform in [Platform::Windows, Platform::Macos, Platform::Linux] {
            for scene in BuiltinScene::ALL {
                let template = scene.template(platform);
                assert_eq!(validate_scene_draft_with(&template, false).as_ref(), Ok(&template), "{scene:?} on {platform:?}");
                assert!(template.matching.apps.len() <= MAX_SCENE_APPS);
                assert!(template.overrides.prompt.as_deref().is_none_or(|p| p.chars().count() <= MAX_SCENE_PROMPT_CHARS));
                assert!(!template.enabled, "every built-in scene starts switched off");
                assert_eq!(BuiltinScene::from_name(&template.name), Some(scene));
                let domain = matches!(scene, BuiltinScene::Legal | BuiltinScene::Medical | BuiltinScene::Finance | BuiltinScene::Academic);
                assert_eq!(template.matching.apps.is_empty(), domain, "{scene:?} on {platform:?}: the domains list no application");
                if domain {
                    assert!(template.overrides.prompt.as_deref().is_some_and(|p| p.starts_with(domain!(""))));
                }
            }
        }
        for platform in [Platform::Android, Platform::Ios, Platform::Other] {
            assert!(BuiltinScene::ALL.iter().all(|s| s.default_apps(platform).is_empty()));
            assert!(!has_builtin_scenes(platform));
        }
        assert_eq!(BuiltinScene::Coding.template(Platform::Windows).overrides.refine_preset, Some(PresetId::Builtin(BuiltinPreset::Proofread)));
        assert_eq!(BuiltinScene::Office.template(Platform::Macos).overrides.refine_preset, Some(PresetId::Builtin(BuiltinPreset::Formal)));
        assert_eq!(
            BuiltinScene::Chat.template(Platform::Linux).overrides,
            SceneOverrides { refine_preset: Some(PresetId::Builtin(BuiltinPreset::Chat)), ..SceneOverrides::default() }
        );
        assert_eq!(serde_json::to_string(&BuiltinScene::Academic).unwrap(), "\"academic\"");
    }
}
