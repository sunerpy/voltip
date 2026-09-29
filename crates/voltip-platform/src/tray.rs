//! The tray icon and its menu (docs/dictation.md §15.4).
//!
//! The icon is the app mark, drawn to RGBA at runtime from the geometry of
//! `packages/ui/src/components/Logo.tsx` (a navy rounded square, a V whose left arm is pale and
//! right arm orange), so it is crisp at whatever size the platform draws and no per-state asset
//! has to be shipped or kept in sync. A badge in the square's bottom-right corner shows the
//! dictation phase. Windows gets the colour mark, the same the window icon shows; macOS gets a
//! *template* (the V alone, black on transparent) that the menu bar tints for its light and dark
//! appearance. The menu's labels come in the UI's two languages.

use crate::HostOs;

/// What the tray shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrayGlyph {
    /// The mark alone: ready, nothing in flight (also the `done` / `failed` / `cancelled` dwell).
    Idle,
    /// A filled badge (red on the colour mark): the microphone is open.
    Listening,
    /// A blue badge on the colour mark, a ring badge on the template: transcribing / refining /
    /// inserting.
    Processing,
}

impl TrayGlyph {
    /// The glyph for a `DictationPhase` wire name (`idle`, `listening`, `processing`, `done`, …).
    /// Unknown names read as idle, so a new phase can never leave a stale "listening" icon.
    pub fn from_phase_name(phase: &str) -> Self {
        match phase {
            "listening" => Self::Listening,
            "processing" => Self::Processing,
            _ => Self::Idle,
        }
    }
}

/// How the mark is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrayStyle {
    /// The full-colour mark (the Windows notification area).
    Color,
    /// A template image (the macOS menu bar): the V alone in black; the menu bar keeps only the
    /// alpha and tints it.
    Template,
}

impl TrayStyle {
    /// The style `os` draws its tray icon in.
    pub const fn for_host(os: HostOs) -> Self {
        match os {
            HostOs::Macos => Self::Template,
            HostOs::Windows | HostOs::Linux | HostOs::Other => Self::Color,
        }
    }
}

/// macOS draws a status item 18 pt tall (tray-icon scales the image to that height): 36 px is one
/// image pixel per screen pixel on a Retina display.
pub const MACOS_TRAY_ICON_SIZE: u32 = 36;
/// Windows draws notification-area icons at the small-icon size (`SM_CXSMICON`: 16 px at 100 %
/// scaling, 32 px at 200 %); this is the size when the shell could not ask.
pub const WINDOWS_TRAY_ICON_SIZE: u32 = 32;
/// The sizes [`tray_icon_size`] hands out; a system metric outside them is clamped.
pub const TRAY_ICON_SIZES: std::ops::RangeInclusive<u32> = 16..=64;

/// The pixel size to render for `os`: on Windows the small-icon size the system reported
/// (`None` when it could not be read), clamped to [`TRAY_ICON_SIZES`], so the shell never scales
/// the image and blurs the mark.
pub fn tray_icon_size(os: HostOs, system_small_icon: Option<u32>) -> u32 {
    match os {
        HostOs::Macos => MACOS_TRAY_ICON_SIZE,
        HostOs::Windows | HostOs::Linux | HostOs::Other => {
            system_small_icon.unwrap_or(WINDOWS_TRAY_ICON_SIZE).clamp(*TRAY_ICON_SIZES.start(), *TRAY_ICON_SIZES.end())
        }
    }
}

// The mark in the 100 × 100 space of `Logo.tsx`.
const NAVY: [u8; 3] = [0x0B, 0x12, 0x20];
const PALE: [u8; 3] = [0xE7, 0xED, 0xF5];
const ORANGE: [u8; 3] = [0xF9, 0x73, 0x16];
const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];
const INK: [u8; 3] = [0, 0, 0];
/// The listening badge on the colour mark: a recording red.
pub const LISTENING_RGB: [u8; 3] = [0xEF, 0x44, 0x44];
/// The processing badge on the colour mark: the app accent.
pub const PROCESSING_RGB: [u8; 3] = [0x2F, 0x6F, 0xED];
const SQUARE_RADIUS: f64 = 22.0;
const PALE_ARM: [(f64, f64); 4] = [(22.5, 22.0), (39.5, 22.0), (50.0, 46.0), (50.0, 79.0)];
const ORANGE_ARM: [(f64, f64); 4] = [(60.5, 22.0), (77.5, 22.0), (50.0, 79.0), (50.0, 46.0)];
/// The colour badge is centred where the square's bottom-right corner arc is, so badge plus its
/// white gap fill that corner exactly.
const BADGE_CENTRE: (f64, f64) = (100.0 - SQUARE_RADIUS, 100.0 - SQUARE_RADIUS);
const BADGE_RADIUS: f64 = 16.0;
const BADGE_GAP_RADIUS: f64 = SQUARE_RADIUS;
/// Without the square the template's V is scaled up about the centre of its box (the mark's
/// 22.5–77.5 × 22–79 becomes 9–91 × 8–93 of the image), so it fills the menu bar's height like
/// other status items.
const TEMPLATE_SCALE: f64 = 1.5;
const TEMPLATE_V_CENTRE: (f64, f64) = (50.0, 50.5);
/// The template badge sits in the image's bottom-right corner, clear of the right arm; its gap is
/// cut out of whatever it overlaps.
const TEMPLATE_BADGE_CENTRE: (f64, f64) = (81.0, 81.0);
const TEMPLATE_BADGE_RADIUS: f64 = 17.0;
/// The hole of the template's processing ring.
const TEMPLATE_BADGE_HOLE_RADIUS: f64 = 9.5;
const TEMPLATE_BADGE_GAP_RADIUS: f64 = 23.0;
/// Samples per pixel side (4 × 4 per pixel) for anti-aliased edges.
const SUBSAMPLES: u32 = 4;

/// Render the mark with `glyph`'s badge as a `size × size` RGBA buffer (row-major, top to bottom,
/// straight alpha). `size` is clamped to 1..=256.
pub fn render_tray_icon(glyph: TrayGlyph, size: u32, style: TrayStyle) -> Vec<u8> {
    let size = size.clamp(1, 256);
    let scale = 100.0 / f64::from(size);
    let step = 1.0 / f64::from(SUBSAMPLES);
    let total = SUBSAMPLES * SUBSAMPLES;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let mut painted = 0u32;
            let mut sum = [0u32; 3];
            for j in 0..SUBSAMPLES {
                for i in 0..SUBSAMPLES {
                    let u = (f64::from(x) + (f64::from(i) + 0.5) * step) * scale;
                    let v = (f64::from(y) + (f64::from(j) + 0.5) * step) * scale;
                    if let Some(rgb) = paint(u, v, glyph, style) {
                        painted += 1;
                        for (acc, channel) in sum.iter_mut().zip(rgb) {
                            *acc += u32::from(channel);
                        }
                    }
                }
            }
            // The painted samples' average colour at their share of the pixel (all zero where
            // nothing was painted).
            for acc in sum {
                out.push((acc + painted / 2).checked_div(painted).and_then(|c| u8::try_from(c).ok()).unwrap_or(0));
            }
            out.push(u8::try_from((painted * 255 + total / 2) / total).unwrap_or(u8::MAX));
        }
    }
    out
}

/// The colour at `(u, v)` of the 100-unit image, or `None` where it is transparent.
fn paint(u: f64, v: f64, glyph: TrayGlyph, style: TrayStyle) -> Option<[u8; 3]> {
    match style {
        TrayStyle::Color => paint_colour(u, v, glyph),
        TrayStyle::Template => paint_template(u, v, glyph),
    }
}

/// Top to bottom: the badge and its white gap, the two arms, the square.
fn paint_colour(u: f64, v: f64, glyph: TrayGlyph) -> Option<[u8; 3]> {
    if glyph != TrayGlyph::Idle {
        let d = distance(u, v, BADGE_CENTRE);
        if d <= BADGE_RADIUS {
            return Some(if glyph == TrayGlyph::Listening { LISTENING_RGB } else { PROCESSING_RGB });
        }
        if d <= BADGE_GAP_RADIUS {
            return in_rounded_square(u, v).then_some(WHITE);
        }
    }
    if in_polygon(u, v, &ORANGE_ARM) {
        Some(ORANGE)
    } else if in_polygon(u, v, &PALE_ARM) {
        Some(PALE)
    } else {
        in_rounded_square(u, v).then_some(NAVY)
    }
}

/// Top to bottom: the badge (a disc, or a ring while processing) and the gap cut around it, the
/// scaled V.
fn paint_template(u: f64, v: f64, glyph: TrayGlyph) -> Option<[u8; 3]> {
    if glyph != TrayGlyph::Idle {
        let d = distance(u, v, TEMPLATE_BADGE_CENTRE);
        if d <= TEMPLATE_BADGE_RADIUS {
            let hole = glyph == TrayGlyph::Processing && d < TEMPLATE_BADGE_HOLE_RADIUS;
            return (!hole).then_some(INK);
        }
        if d <= TEMPLATE_BADGE_GAP_RADIUS {
            return None;
        }
    }
    let mu = TEMPLATE_V_CENTRE.0 + (u - TEMPLATE_V_CENTRE.0) / TEMPLATE_SCALE;
    let mv = TEMPLATE_V_CENTRE.1 + (v - TEMPLATE_V_CENTRE.1) / TEMPLATE_SCALE;
    (in_polygon(mu, mv, &ORANGE_ARM) || in_polygon(mu, mv, &PALE_ARM)).then_some(INK)
}

fn distance(u: f64, v: f64, (cu, cv): (f64, f64)) -> f64 {
    ((u - cu).powi(2) + (v - cv).powi(2)).sqrt()
}

fn in_rounded_square(u: f64, v: f64) -> bool {
    if !(0.0..=100.0).contains(&u) || !(0.0..=100.0).contains(&v) {
        return false;
    }
    // Distance past the straight edges into a corner's square, per axis.
    let dx = (SQUARE_RADIUS - u).max(u - (100.0 - SQUARE_RADIUS)).max(0.0);
    let dy = (SQUARE_RADIUS - v).max(v - (100.0 - SQUARE_RADIUS)).max(0.0);
    dx * dx + dy * dy <= SQUARE_RADIUS * SQUARE_RADIUS
}

/// Even-odd rule.
fn in_polygon(u: f64, v: f64, polygon: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut previous = polygon.last().copied().unwrap_or_default();
    for &(x, y) in polygon {
        let (px, py) = previous;
        if (y > v) != (py > v) && u < (px - x) * (v - y) / (py - y) + x {
            inside = !inside;
        }
        previous = (x, y);
    }
    inside
}

/// The tray menu's language: the UI's two locales (`packages/shared/src/i18n`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TrayLocale {
    /// 简体中文, the product's first language: what the menu speaks until it knows better.
    #[default]
    ZhCn,
    /// English.
    En,
}

impl TrayLocale {
    /// The locale the webview resolved (`zh-CN` / `en`, TS `Locale`).
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "zh-CN" => Some(Self::ZhCn),
            "en" => Some(Self::En),
            _ => None,
        }
    }

    /// What `settings.locale = "system"` resolves to for an OS language tag: Chinese for any
    /// `zh…`, English otherwise (TS `resolveLocale`).
    pub fn for_language(language: &str) -> Self {
        if language.trim().to_ascii_lowercase().starts_with("zh") { Self::ZhCn } else { Self::En }
    }
}

/// The tray menu's entries, in menu order; a separator goes before [`TrayAction::Quit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrayAction {
    /// Bring the main window to the front.
    Open,
    /// The main window with the settings dialog.
    Settings,
    /// The main window with the update dialog; only in a build that has an update source.
    CheckUpdate,
    /// Quit Voltip.
    Quit,
}

impl TrayAction {
    /// Every entry, in menu order.
    pub const ALL: [Self; 4] = [Self::Open, Self::Settings, Self::CheckUpdate, Self::Quit];

    /// The menu item id.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Open => "tray-open",
            Self::Settings => "tray-settings",
            Self::CheckUpdate => "tray-check-update",
            Self::Quit => "tray-quit",
        }
    }

    /// The entry a menu event names; `None` for ids that are not the tray's (the handler sees
    /// every menu event of the app).
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }

    /// The label in `locale`, worded like the UI (`docs/frontend.md` §6.4: one language per
    /// locale).
    pub const fn label(self, locale: TrayLocale) -> &'static str {
        match (self, locale) {
            (Self::Open, TrayLocale::ZhCn) => "打开 Voltip",
            (Self::Open, TrayLocale::En) => "Open Voltip",
            (Self::Settings, TrayLocale::ZhCn) => "设置…",
            (Self::Settings, TrayLocale::En) => "Settings…",
            (Self::CheckUpdate, TrayLocale::ZhCn) => "检查更新…",
            (Self::CheckUpdate, TrayLocale::En) => "Check for Updates…",
            (Self::Quit, TrayLocale::ZhCn) => "退出 Voltip",
            (Self::Quit, TrayLocale::En) => "Quit Voltip",
        }
    }

    /// What the main window's webview does once the window is up (the `voltip://tray` event's
    /// `action`); `None` when showing the window is all there is.
    pub const fn webview_action(self) -> Option<&'static str> {
        match self {
            Self::Settings => Some("settings"),
            Self::CheckUpdate => Some("update"),
            Self::Open | Self::Quit => None,
        }
    }

    /// Whether the entry belongs in the menu of a build that can (`updater`) or cannot update.
    pub const fn shown(self, updater: bool) -> bool {
        match self {
            Self::CheckUpdate => updater,
            Self::Open | Self::Settings | Self::Quit => true,
        }
    }
}

/// The AI 润色 submenu (docs/dictation.md §21): the switch, then every preset (the built-in ones
/// in the menu's language, the custom ones by their own names), the one the settings name checked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayPolish {
    /// Whether the clean-up runs.
    pub enabled: bool,
    /// The presets, in the interface's order.
    pub presets: Vec<TrayPreset>,
}

/// One preset of [`TrayPolish`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayPreset {
    /// The preset's wire id: a built-in name or a custom preset's UUID.
    pub id: String,
    /// What the entry says.
    pub label: String,
    /// The preset the settings name.
    pub checked: bool,
}

/// The AI 润色 submenu's id.
pub const TRAY_POLISH_ID: &str = "tray-polish";
/// Its switch's id.
pub const TRAY_POLISH_TOGGLE_ID: &str = "tray-polish-toggle";
const TRAY_PRESET_PREFIX: &str = "tray-preset:";

/// The menu item id of the preset `id` names.
pub fn tray_preset_id(id: &str) -> String {
    format!("{TRAY_PRESET_PREFIX}{id}")
}

/// An entry of the AI 润色 submenu, as a menu event names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayPolishAction<'a> {
    /// The switch.
    Toggle,
    /// A preset, by its wire id.
    Preset(&'a str),
}

impl<'a> TrayPolishAction<'a> {
    /// The entry a menu event names; `None` for ids that are not the submenu's.
    pub fn from_id(id: &'a str) -> Option<Self> {
        if id == TRAY_POLISH_TOGGLE_ID {
            return Some(Self::Toggle);
        }
        id.strip_prefix(TRAY_PRESET_PREFIX).filter(|preset| !preset.is_empty()).map(Self::Preset)
    }
}

/// The submenu's label.
pub const fn polish_menu_label(locale: TrayLocale) -> &'static str {
    match locale {
        TrayLocale::ZhCn => "AI 润色",
        TrayLocale::En => "AI Polish",
    }
}

/// The switch's label (checked while the clean-up runs).
pub const fn polish_toggle_label(locale: TrayLocale) -> &'static str {
    match locale {
        TrayLocale::ZhCn => "启用 AI 润色",
        TrayLocale::En => "Enable AI Polish",
    }
}

/// A built-in preset's name in the menu's language, worded like the interface (`presets.<id>.name`
/// of `packages/shared/src/i18n`); `None` for a name that is no built-in preset's.
pub fn builtin_preset_label(id: &str, locale: TrayLocale) -> Option<&'static str> {
    let zh = locale == TrayLocale::ZhCn;
    Some(match id {
        "proofread" => {
            if zh {
                "校对"
            } else {
                "Proofread"
            }
        }
        "prompt" => {
            if zh {
                "提示词优化"
            } else {
                "Prompt optimizer"
            }
        }
        "intent" => {
            if zh {
                "意图整理"
            } else {
                "Clarify intent"
            }
        }
        "chat" => {
            if zh {
                "口语聊天"
            } else {
                "Casual chat"
            }
        }
        "translate" => {
            if zh {
                "中英互译"
            } else {
                "Chinese ⇄ English"
            }
        }
        "notes" => {
            if zh {
                "要点纪要"
            } else {
                "Key points"
            }
        }
        "punctuation" => {
            if zh {
                "只加标点"
            } else {
                "Punctuation only"
            }
        }
        "formal" => {
            if zh {
                "书面语"
            } else {
                "Formal"
            }
        }
        _ => return None,
    })
}

/// The tray tooltip: the product name, plus the phase while one is in flight.
pub const fn tray_tooltip(glyph: TrayGlyph, locale: TrayLocale) -> &'static str {
    match (glyph, locale) {
        (TrayGlyph::Idle, _) => "Voltip",
        (TrayGlyph::Listening, TrayLocale::ZhCn) => "Voltip · 正在听写",
        (TrayGlyph::Listening, TrayLocale::En) => "Voltip · Listening",
        (TrayGlyph::Processing, TrayLocale::ZhCn) => "Voltip · 正在处理",
        (TrayGlyph::Processing, TrayLocale::En) => "Voltip · Processing",
    }
}

/// What closing the main window does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CloseAction {
    /// Hide it: Voltip keeps running (hotkey, tray, Dock), and the tray, the Dock or a second
    /// launch brings the window back.
    Hide,
    /// Quit Voltip. Letting the window close without quitting would leave the process running
    /// with nothing to bring the window back: the prewarmed pill window keeps the event loop alive.
    Quit,
}

/// macOS always hides (the Dock brings the window back, as on every Mac app); Windows hides while
/// its tray icon is up and quits without one; Linux (no tray) quits.
pub const fn main_window_close(os: HostOs, tray_installed: bool) -> CloseAction {
    match os {
        HostOs::Macos => CloseAction::Hide,
        HostOs::Windows if tray_installed => CloseAction::Hide,
        HostOs::Windows | HostOs::Linux | HostOs::Other => CloseAction::Quit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZES: [u32; 5] = [16, 20, 24, 32, 36];

    fn pixel(buf: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    /// The pixel under a point of the 100-unit mark.
    fn at(buf: &[u8], size: u32, u: f64, v: f64) -> [u8; 4] {
        let to_px = |c: f64| ((c / 100.0 * f64::from(size)).floor() as u32).min(size - 1);
        pixel(buf, size, to_px(u), to_px(v))
    }

    /// Deep inside each arm (the arms are mirror images about u = 50).
    const PALE_INSIDE: (f64, f64) = (38.0, 42.0);
    const ORANGE_INSIDE: (f64, f64) = (62.0, 42.0);

    /// Where a point of the mark lands in the scaled-up template.
    fn template_point((u, v): (f64, f64)) -> (f64, f64) {
        (TEMPLATE_V_CENTRE.0 + (u - TEMPLATE_V_CENTRE.0) * TEMPLATE_SCALE, TEMPLATE_V_CENTRE.1 + (v - TEMPLATE_V_CENTRE.1) * TEMPLATE_SCALE)
    }

    fn colour_distance(a: [u8; 4], b: [u8; 3]) -> u32 {
        a.iter().zip(b).map(|(&x, y)| u32::from(x.abs_diff(y)).pow(2)).sum()
    }

    /// At 16–24 px an arm is two or three pixels wide, so the pixel under a point inside it may
    /// be partly square: it must still be opaque and nearer the arm's colour than the navy's.
    fn arm_colour(buf: &[u8], size: u32, (u, v): (f64, f64), colour: [u8; 3]) {
        let p = at(buf, size, u, v);
        if size >= 32 {
            assert_eq!(p, [colour[0], colour[1], colour[2], 255], "{size}: ({u}, {v})");
        } else {
            assert_eq!(p[3], 255, "{size}: ({u}, {v}) {p:?}");
            assert!(colour_distance(p, colour) < colour_distance(p, NAVY), "{size}: ({u}, {v}) {p:?} is not {colour:?}");
        }
    }

    #[test]
    fn glyphs_follow_the_phase_name() {
        assert_eq!(TrayGlyph::from_phase_name("listening"), TrayGlyph::Listening);
        assert_eq!(TrayGlyph::from_phase_name("processing"), TrayGlyph::Processing);
        for idle in ["idle", "done", "failed", "cancelled", "", "something_new"] {
            assert_eq!(TrayGlyph::from_phase_name(idle), TrayGlyph::Idle, "{idle}");
        }
    }

    /// Regression (user report 2026-09-28): the Windows tray showed a blue ring instead of the
    /// logo. The colour icon is the mark: navy square, pale left arm, orange right arm, and
    /// transparent only outside the rounded corners.
    #[test]
    fn the_colour_icon_is_the_logo() {
        for size in SIZES {
            let idle = render_tray_icon(TrayGlyph::Idle, size, TrayStyle::Color);
            assert_eq!(idle.len(), (size * size * 4) as usize);
            assert_eq!(at(&idle, size, 50.0, 10.0), [NAVY[0], NAVY[1], NAVY[2], 255], "{size}: square");
            arm_colour(&idle, size, PALE_INSIDE, PALE);
            arm_colour(&idle, size, ORANGE_INSIDE, ORANGE);
            assert_eq!(pixel(&idle, size, 0, 0)[3], 0, "{size}: the rounded corner is transparent");
            assert_eq!(pixel(&idle, size, size - 1, size - 1)[3], 0, "{size}");
            // The old ring was one colour everywhere; the mark has three.
            let colours: std::collections::HashSet<[u8; 3]> = idle.as_chunks::<4>().0.iter().filter(|p| p[3] == 255).map(|p| [p[0], p[1], p[2]]).collect();
            for colour in [NAVY, PALE, ORANGE] {
                assert!(colours.contains(&colour), "{size}: {colour:?} missing");
            }
            assert!(!colours.contains(&PROCESSING_RGB), "{size}: no accent-blue ring any more");
        }
    }

    #[test]
    fn the_colour_badge_shows_the_phase_in_the_corner() {
        for size in SIZES {
            let idle = render_tray_icon(TrayGlyph::Idle, size, TrayStyle::Color);
            let listening = render_tray_icon(TrayGlyph::Listening, size, TrayStyle::Color);
            let processing = render_tray_icon(TrayGlyph::Processing, size, TrayStyle::Color);
            let (cu, cv) = BADGE_CENTRE;
            assert_eq!(at(&idle, size, cu, cv), [NAVY[0], NAVY[1], NAVY[2], 255], "{size}");
            assert_eq!(at(&listening, size, cu, cv), [LISTENING_RGB[0], LISTENING_RGB[1], LISTENING_RGB[2], 255], "{size}");
            assert_eq!(at(&processing, size, cu, cv), [PROCESSING_RGB[0], PROCESSING_RGB[1], PROCESSING_RGB[2], 255], "{size}");
            // The white gap separates the badge from the navy square (sampled halfway between the
            // badge edge and the corner arc, above the centre).
            if size >= 24 {
                assert_eq!(at(&listening, size, cu, cv - (BADGE_RADIUS + BADGE_GAP_RADIUS) / 2.0), [255, 255, 255, 255], "{size}");
            }
            // Away from the corner nothing changes.
            for (u, v) in [(50.0, 10.0), PALE_INSIDE, ORANGE_INSIDE, (10.0, 90.0)] {
                assert_eq!(at(&listening, size, u, v), at(&idle, size, u, v), "{size}: ({u}, {v})");
            }
        }
    }

    #[test]
    fn the_template_is_the_v_alone_in_ink() {
        for size in SIZES {
            let idle = render_tray_icon(TrayGlyph::Idle, size, TrayStyle::Template);
            // Only alpha counts in a template: every pixel is ink or transparent.
            assert!(idle.as_chunks::<4>().0.iter().all(|p| p[..3] == INK || p[3] == 0), "{size}");
            for (u, v) in [PALE_INSIDE, ORANGE_INSIDE].map(template_point) {
                let alpha = at(&idle, size, u, v)[3];
                assert!(if size >= 32 { alpha == 255 } else { alpha >= 192 }, "{size}: arm at ({u}, {v}) alpha {alpha}");
            }
            // No square: the background between and around the arms is clear.
            assert_eq!(at(&idle, size, 50.0, 12.0)[3], 0, "{size}");
            assert_eq!(at(&idle, size, 8.0, 92.0)[3], 0, "{size}");
            // Scaled up: the arms reach close to the top edge and the tip close to the bottom.
            let rows_with_ink: Vec<u32> = (0..size).filter(|&y| (0..size).any(|x| pixel(&idle, size, x, y)[3] > 0)).collect();
            let (top, bottom) = (rows_with_ink[0], rows_with_ink[rows_with_ink.len() - 1]);
            assert!(f64::from(bottom - top + 1) >= 0.8 * f64::from(size), "{size}: the V spans {top}..={bottom}");
            let listening = render_tray_icon(TrayGlyph::Listening, size, TrayStyle::Template);
            let processing = render_tray_icon(TrayGlyph::Processing, size, TrayStyle::Template);
            let (cu, cv) = TEMPLATE_BADGE_CENTRE;
            assert_eq!(at(&idle, size, cu, cv)[3], 0, "{size}");
            assert_eq!(at(&listening, size, cu, cv)[3], 255, "{size}: filled badge");
            if size >= 32 {
                assert_eq!(at(&processing, size, cu, cv)[3], 0, "{size}: the ring's hole");
                let ring = (TEMPLATE_BADGE_RADIUS + TEMPLATE_BADGE_HOLE_RADIUS) / 2.0;
                assert_eq!(at(&processing, size, cu, cv - ring)[3], 255, "{size}: the ring");
            }
        }
    }

    #[test]
    fn edges_are_anti_aliased() {
        let icon = render_tray_icon(TrayGlyph::Idle, 32, TrayStyle::Template);
        assert!(icon.as_chunks::<4>().0.iter().any(|p| p[3] > 0 && p[3] < 255), "partial coverage on the arms' slanted edges");
    }

    #[test]
    fn sizes_are_clamped() {
        for size in [0, 1, 2, 3] {
            let buf = render_tray_icon(TrayGlyph::Processing, size, TrayStyle::Color);
            assert_eq!(buf.len(), (size.max(1) * size.max(1) * 4) as usize);
        }
        assert_eq!(render_tray_icon(TrayGlyph::Idle, 10_000, TrayStyle::Color).len(), 256 * 256 * 4);
        assert_eq!(tray_icon_size(HostOs::Macos, Some(16)), MACOS_TRAY_ICON_SIZE);
        assert_eq!(tray_icon_size(HostOs::Windows, None), WINDOWS_TRAY_ICON_SIZE);
        assert_eq!(tray_icon_size(HostOs::Windows, Some(20)), 20);
        assert_eq!(tray_icon_size(HostOs::Windows, Some(4)), 16);
        assert_eq!(tray_icon_size(HostOs::Windows, Some(512)), 64);
        assert_eq!(TrayStyle::for_host(HostOs::Macos), TrayStyle::Template);
        assert_eq!(TrayStyle::for_host(HostOs::Windows), TrayStyle::Color);
    }

    /// Regression (user report 2026-09-28): the tray had no menu, so there was no way to quit.
    #[test]
    fn the_menu_opens_the_window_settings_updates_and_quits() {
        assert_eq!(TrayAction::ALL.map(TrayAction::id), ["tray-open", "tray-settings", "tray-check-update", "tray-quit"]);
        for action in TrayAction::ALL {
            assert_eq!(TrayAction::from_id(action.id()), Some(action));
            assert!(action.label(TrayLocale::ZhCn).chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)), "{action:?}");
            assert!(action.label(TrayLocale::En).is_ascii() || action.label(TrayLocale::En).ends_with('…'), "{action:?}");
            assert!(!action.label(TrayLocale::En).chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)), "{action:?}: no CJK in English");
        }
        assert_eq!(TrayAction::from_id("quit"), None, "only the tray's own ids");
        assert_eq!(TrayAction::Quit.label(TrayLocale::ZhCn), "退出 Voltip");
        assert_eq!(TrayAction::Open.label(TrayLocale::En), "Open Voltip");
        assert!(TrayAction::ALL.iter().all(|a| a.shown(true)));
        assert!(!TrayAction::CheckUpdate.shown(false));
        assert!(TrayAction::Quit.shown(false), "quit is always there");
        assert_eq!(TrayAction::Settings.webview_action(), Some("settings"));
        assert_eq!(TrayAction::CheckUpdate.webview_action(), Some("update"));
        assert_eq!(TrayAction::Open.webview_action(), None);
        assert_eq!(TrayAction::Quit.webview_action(), None);
    }

    #[test]
    fn the_menu_speaks_the_ui_language() {
        assert_eq!(TrayLocale::from_tag("zh-CN"), Some(TrayLocale::ZhCn));
        assert_eq!(TrayLocale::from_tag("en"), Some(TrayLocale::En));
        assert_eq!(TrayLocale::from_tag("fr"), None);
        for (language, locale) in [
            ("zh-CN", TrayLocale::ZhCn),
            ("zh-Hans-CN", TrayLocale::ZhCn),
            ("ZH-TW", TrayLocale::ZhCn),
            ("en-US", TrayLocale::En),
            ("de", TrayLocale::En),
            ("", TrayLocale::En),
        ] {
            assert_eq!(TrayLocale::for_language(language), locale, "{language}");
        }
        assert_eq!(TrayLocale::default(), TrayLocale::ZhCn);
        assert_eq!(tray_tooltip(TrayGlyph::Idle, TrayLocale::En), "Voltip");
        assert_eq!(tray_tooltip(TrayGlyph::Listening, TrayLocale::ZhCn), "Voltip · 正在听写");
        assert_eq!(tray_tooltip(TrayGlyph::Processing, TrayLocale::En), "Voltip · Processing");
    }

    /// Regression (user report 2026-09-28): closing the window destroyed it while the prewarmed
    /// pill window kept the process alive, so the tray (and a second launch) had nothing to show.
    #[test]
    fn the_polish_submenu_names_its_entries_and_reads_them_back() {
        assert_eq!(TrayPolishAction::from_id(TRAY_POLISH_TOGGLE_ID), Some(TrayPolishAction::Toggle));
        assert_eq!(TrayPolishAction::from_id(&tray_preset_id("notes")), Some(TrayPolishAction::Preset("notes")));
        let custom = "7e57ab1e-0b0e-4c0d-9e5e-7e57ab1e0b0e";
        assert_eq!(TrayPolishAction::from_id(&tray_preset_id(custom)), Some(TrayPolishAction::Preset(custom)));
        for other in ["tray-preset:", "tray-quit", TRAY_POLISH_ID, "preset:notes"] {
            assert_eq!(TrayPolishAction::from_id(other), None, "{other}");
        }
        assert_eq!((polish_menu_label(TrayLocale::ZhCn), polish_menu_label(TrayLocale::En)), ("AI 润色", "AI Polish"));
        assert_eq!(polish_toggle_label(TrayLocale::En), "Enable AI Polish");
        for id in ["proofread", "prompt", "intent", "chat", "translate", "notes", "punctuation", "formal"] {
            let zh = builtin_preset_label(id, TrayLocale::ZhCn).unwrap();
            let en = builtin_preset_label(id, TrayLocale::En).unwrap();
            assert!(zh.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)), "{id}: {zh}");
            assert!(!en.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)), "{id}: no CJK in English");
        }
        assert_eq!(builtin_preset_label("default", TrayLocale::ZhCn), None);
    }

    #[test]
    fn closing_the_main_window_hides_it_where_something_brings_it_back() {
        let table = [
            (HostOs::Macos, true, CloseAction::Hide),
            (HostOs::Macos, false, CloseAction::Hide),
            (HostOs::Windows, true, CloseAction::Hide),
            (HostOs::Windows, false, CloseAction::Quit),
            (HostOs::Linux, false, CloseAction::Quit),
            (HostOs::Linux, true, CloseAction::Quit),
            (HostOs::Other, false, CloseAction::Quit),
        ];
        for (os, tray, action) in table {
            assert_eq!(main_window_close(os, tray), action, "{os:?} tray={tray}");
        }
    }
}
