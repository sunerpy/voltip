//! The tray icon and its menu (docs/dictation.md §15.4).
//!
//! The icon is the app mark 「声波光标」, drawn to RGBA at runtime from the geometry of
//! `packages/ui/src/components/Logo.tsx` (on a deep ink rounded square, three white sound bars run
//! into a cyan text cursor), so it is crisp at whatever size the platform draws and no per-state
//! asset has to be shipped or kept in sync. A badge in the square's bottom-right corner shows the
//! dictation phase. Windows gets the colour mark, the same the window icon shows; macOS gets a
//! *template* (the bars and the cursor alone, black on transparent) that the menu bar tints for its
//! light and dark appearance. The menu's labels come in the UI's two languages.

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
    /// A template image (the macOS menu bar): the bars and the cursor alone in black; the menu bar
    /// keeps only the alpha and tints it.
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

// The image is a 100 × 100 space: `Logo.tsx`'s 1024 canvas scaled by 100 / 1024.
const K: f64 = 100.0 / 1024.0;
/// The tile's gradient, top left to bottom right.
const TILE_FROM: [u8; 3] = [0x0B, 0x12, 0x20];
const TILE_TO: [u8; 3] = [0x1B, 0x2A, 0x4A];
const WAVE: [u8; 3] = [0xFF, 0xFF, 0xFF];
/// The cursor's gradient, top to bottom.
const CURSOR_FROM: [u8; 3] = [0x38, 0xBD, 0xF8];
const CURSOR_TO: [u8; 3] = [0x22, 0xD3, 0xEE];
const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];
const INK: [u8; 3] = [0, 0, 0];
/// The listening badge on the colour mark: a recording red.
pub const LISTENING_RGB: [u8; 3] = [0xEF, 0x44, 0x44];
/// The processing badge on the colour mark: the app accent.
pub const PROCESSING_RGB: [u8; 3] = [0x2F, 0x6F, 0xED];
const SQUARE_RADIUS: f64 = 232.0 * K;
// The bars and the cursor on the 1024 canvas: three bars 84 wide, 48 apart, then 116 to the
// cursor, 72 wide; every part is centred on the canvas's middle.
const BAR_WIDTH: f64 = 84.0;
const BAR_GAP: f64 = 48.0;
const CURSOR_WIDTH: f64 = 72.0;
const CURSOR_GAP: f64 = 116.0;
/// The bars' heights, left to right.
const BAR_HEIGHTS: [f64; 3] = [220.0, 420.0, 300.0];
const CURSOR_HEIGHT: f64 = 560.0;
/// A bar or the cursor: left, top, width, height in the 100-unit image; both ends fully round.
type Pill = (f64, f64, f64, f64);
/// The colour badge is centred where the square's bottom-right corner arc is, so badge plus its
/// white gap fill that corner exactly (it covers the cursor's lower end while it shows).
const BADGE_CENTRE: (f64, f64) = (100.0 - SQUARE_RADIUS, 100.0 - SQUARE_RADIUS);
const BADGE_RADIUS: f64 = 16.0;
const BADGE_GAP_RADIUS: f64 = SQUARE_RADIUS;
/// Without the square the template's bars and cursor are scaled up about the image's centre (the
/// mark's 23.8–76.2 × 22.7–77.3 becomes about 9.5–90.5 × 7.6–92.4 of the image), so they fill the
/// menu bar's height like other status items.
const TEMPLATE_SCALE: f64 = 1.55;
/// The template badge sits in the image's bottom-right corner, over the cursor's lower end; its gap
/// is cut out of whatever it overlaps.
const TEMPLATE_BADGE_CENTRE: (f64, f64) = (81.0, 81.0);
const TEMPLATE_BADGE_RADIUS: f64 = 17.0;
/// The hole of the template's processing ring.
const TEMPLATE_BADGE_HOLE_RADIUS: f64 = 9.5;
const TEMPLATE_BADGE_GAP_RADIUS: f64 = 23.0;
/// Samples per pixel side (8 × 8 per pixel) for the anti-aliased round ends and corners.
const SUBSAMPLES: u32 = 8;

/// The bars and the cursor of one image size.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Mark {
    bars: [Pill; 3],
    cursor: Pill,
}

impl Mark {
    /// The mark scaled by `scale` about the centre of a `size`-pixel image and fitted to its whole
    /// pixels, as `fit` in `scripts/render-icons.py` fits the app icon: the bar width, the gaps and
    /// the heights are whole pixels, every straight edge lies on a pixel boundary and each part
    /// stays centred. Drawn as designed, a bar one to three pixels wide has its edges between
    /// pixels and smears into a grey band (user 2026-10-08: 清晰度要提高下); fitted, only its round
    /// ends are anti-aliased. At 1024 px the fit is the design itself.
    fn fitted(size: u32, scale: f64) -> Self {
        let side = i64::from(size);
        let px = f64::from(size) / 1024.0 * scale;
        let (w_t, g_t, c_t, g2_t) = (BAR_WIDTH * px, BAR_GAP * px, CURSOR_WIDTH * px, CURSOR_GAP * px);
        let span = (3.0 * BAR_WIDTH + 2.0 * BAR_GAP + CURSOR_GAP + CURSOR_WIDTH) * px;
        // Bar width, bar gap, cursor width, cursor gap, total width.
        let mut best: Option<(f64, [i64; 5])> = None;
        for w in near(w_t) {
            for g in near(g_t) {
                for wc in near(c_t).filter(|&wc| wc <= w) {
                    let low = (g + 1).max(whole(g2_t.floor()) - 1);
                    for g2 in low..=(low + 1).max(whole(g2_t.ceil()) + 1) {
                        let total = 3 * w + 2 * g + g2 + wc;
                        if (side - total) % 2 != 0 {
                            continue; // it could not sit centred on whole pixels
                        }
                        let [wf, gf, wcf, g2f, totalf] = [w, g, wc, g2, total].map(|n| n as f64);
                        // The bar width against its target, the gap and the cursor's width as
                        // shares of the bar width (what the eye compares), the cursor's distance,
                        // and the whole span.
                        let cost = 3.0 * ((wf - w_t) / w_t).powi(2)
                            + 2.0 * ((gf / wf - BAR_GAP / BAR_WIDTH) / (BAR_GAP / BAR_WIDTH)).powi(2)
                            + ((wcf / wf - CURSOR_WIDTH / BAR_WIDTH) / (CURSOR_WIDTH / BAR_WIDTH)).powi(2)
                            + ((g2f - g2_t) / g2_t).powi(2)
                            + 8.0 * ((totalf - span) / span).powi(2);
                        if best.is_none_or(|(least, _)| cost < least) {
                            best = Some((cost, [w, g, wc, g2, total]));
                        }
                    }
                }
            }
        }
        // Some cursor gap of the two or more tried always has the image's parity.
        let [w, g, wc, g2, total] = best.map_or([1, 1, 1, 2, 7], |(_, fit)| fit);
        // Heights take the image's parity, so each part is centred on whole pixels, and keep their
        // order: short bar < middle bar < tall bar < cursor.
        let mut order = [BAR_HEIGHTS[0], BAR_HEIGHTS[2], BAR_HEIGHTS[1], CURSOR_HEIGHT];
        order.sort_by(f64::total_cmp);
        let floors = [w, w, w, wc];
        let options: Vec<Vec<i64>> = order
            .iter()
            .zip(floors)
            .map(|(&height, floor)| {
                let t = whole((height * px).floor());
                (floor.max(t - 4)..=t + 5).filter(|v| (v - side) % 2 == 0).collect()
            })
            .collect();
        let mut best_heights: Option<(f64, [i64; 4])> = None;
        for &a in &options[0] {
            for &b in options[1].iter().filter(|&&b| b > a) {
                for &c in options[2].iter().filter(|&&c| c > b) {
                    for &d in options[3].iter().filter(|&&d| d > c) {
                        let heights = [a, b, c, d];
                        let cost: f64 = heights.iter().zip(order).map(|(&h, target)| ((h as f64 - target * px) / (target * px)).powi(2)).sum();
                        if best_heights.is_none_or(|(least, _)| cost < least) {
                            best_heights = Some((cost, heights));
                        }
                    }
                }
            }
        }
        let heights = best_heights.map_or([w, w + 2, w + 4, w + 6], |(_, heights)| heights);
        let height_of = |design: f64| order.iter().position(|&h| h == design).map_or(w, |i| heights[i]);
        let k = 100.0 / f64::from(size);
        let pill = |left: i64, width: i64, height: i64| (left as f64 * k, ((side - height) / 2) as f64 * k, width as f64 * k, height as f64 * k);
        let left = (side - total) / 2;
        let bars = [0, 1, 2].map(|i| pill(left + i * (w + g), w, height_of(BAR_HEIGHTS[i as usize])));
        Self { bars, cursor: pill(left + 3 * w + 2 * g + g2, wc, height_of(CURSOR_HEIGHT)) }
    }

    fn contains(&self, u: f64, v: f64) -> bool {
        in_pill(u, v, self.cursor) || self.bars.iter().any(|&bar| in_pill(u, v, bar))
    }
}

/// The whole numbers either side of `target`, at least 1.
fn near(target: f64) -> std::ops::RangeInclusive<i64> {
    whole(target.floor()).max(1)..=whole(target.ceil()).max(1)
}

/// An already whole `f64` (a floor or a ceiling of a pixel count) as an `i64`.
fn whole(value: f64) -> i64 {
    value as i64
}

/// Render the mark with `glyph`'s badge as a `size × size` RGBA buffer (row-major, top to bottom,
/// straight alpha). `size` is clamped to 1..=256.
pub fn render_tray_icon(glyph: TrayGlyph, size: u32, style: TrayStyle) -> Vec<u8> {
    let size = size.clamp(1, 256);
    let mark = Mark::fitted(size, if style == TrayStyle::Template { TEMPLATE_SCALE } else { 1.0 });
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
                    let rgb = match style {
                        TrayStyle::Color => paint_colour(u, v, glyph, &mark),
                        TrayStyle::Template => paint_template(u, v, glyph, &mark),
                    };
                    if let Some(rgb) = rgb {
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

/// Top to bottom: the badge and its white gap, the cursor, the bars, the square.
fn paint_colour(u: f64, v: f64, glyph: TrayGlyph, mark: &Mark) -> Option<[u8; 3]> {
    if glyph != TrayGlyph::Idle {
        let d = distance(u, v, BADGE_CENTRE);
        if d <= BADGE_RADIUS {
            return Some(if glyph == TrayGlyph::Listening { LISTENING_RGB } else { PROCESSING_RGB });
        }
        if d <= BADGE_GAP_RADIUS {
            return in_rounded_square(u, v).then_some(WHITE);
        }
    }
    if in_pill(u, v, mark.cursor) {
        let (_, top, _, height) = mark.cursor;
        Some(mix(CURSOR_FROM, CURSOR_TO, (v - top) / height))
    } else if mark.bars.iter().any(|&bar| in_pill(u, v, bar)) {
        Some(WAVE)
    } else {
        // The tile's gradient runs along the diagonal: 0 at the top left, 1 at the bottom right.
        in_rounded_square(u, v).then(|| mix(TILE_FROM, TILE_TO, (u + v) / 200.0))
    }
}

/// Top to bottom: the badge (a disc, or a ring while processing) and the gap cut around it, the
/// scaled-up bars and cursor.
fn paint_template(u: f64, v: f64, glyph: TrayGlyph, mark: &Mark) -> Option<[u8; 3]> {
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
    mark.contains(u, v).then_some(INK)
}

/// `from` blended toward `to` by `t` (clamped to 0..=1), per channel.
fn mix(from: [u8; 3], to: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let mut out = [0u8; 3];
    for ((o, a), b) in out.iter_mut().zip(from).zip(to) {
        let c = f64::from(a) + (f64::from(b) - f64::from(a)) * t;
        *o = u8::try_from(c.round() as i64).unwrap_or(u8::MAX);
    }
    out
}

/// A vertical bar with fully round ends: a rectangle `(left, top, width, height)` whose top and
/// bottom are half circles of the width.
fn in_pill(u: f64, v: f64, (left, top, width, height): Pill) -> bool {
    let r = width / 2.0;
    let cx = left + r;
    // The nearest point of the bar's centre line, from the top cap's centre to the bottom one's.
    let cy = v.clamp(top + r, top + height - r);
    distance(u, v, (cx, cy)) <= r
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

/// Rebuilds a tray menu in the order the rebuilds read their state. A menu choice forces a rebuild
/// (a clicked check item toggles itself, and the menu must show what was saved), and the settings
/// event the choice causes triggers another; the two run at the same time. Reading the state,
/// comparing it with the menu shown, building and installing the new menu all happen under one
/// lock, so the menu installed last is built from the newest state. (CI 2026-09-29, the macOS tray
/// smoke: the forced rebuild had read the state before AI 润色 was switched back on and installed
/// its menu 37 µs after the one built from the new state; the menu showed the switch off.)
#[derive(Debug)]
pub struct MenuSync<M> {
    shown: std::sync::Mutex<M>,
}

impl<M: Clone + PartialEq> MenuSync<M> {
    /// `initial`: the model of the menu the tray was created with.
    pub fn new(initial: M) -> Self {
        Self { shown: std::sync::Mutex::new(initial) }
    }

    /// Rebuild when the model `read` returns differs from the one shown, or always with `force`.
    /// `read` gets the model shown (for what it does not read itself); `install` builds and installs
    /// the menu. Both run under the lock. `Ok(true)`: a menu was installed. After an error the model
    /// shown stays as it was, so the next rebuild tries again.
    pub fn sync<E>(&self, force: bool, read: impl FnOnce(&M) -> M, install: impl FnOnce(&M) -> Result<(), E>) -> Result<bool, E> {
        let mut shown = self.shown.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let next = read(&shown);
        if !force && *shown == next {
            return Ok(false);
        }
        install(&next)?;
        *shown = next;
        Ok(true)
    }
}

/// Tells a single left click on the menu bar item from a double click (user request 2026-09-30:
/// a double click opens the main window). macOS reports clicks only (tray-icon's `DoubleClick` is
/// Windows-only), so a first click waits for the system's double-click interval: a second click
/// within it opens the window, otherwise the menu opens when the wait ends. Times are milliseconds
/// on any monotonic clock the caller keeps.
#[derive(Debug, Default)]
pub struct ClickSeries {
    /// The click that waits, and the generation its timer was armed with.
    pending: Option<(u64, u64)>,
    generation: u64,
}

/// What a click asks the shell to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickStep {
    /// Arm a timer of `after_ms`; when it fires, call [`ClickSeries::expire`] with `generation`.
    Wait {
        /// Names this wait; a later click makes it stale.
        generation: u64,
        /// The double-click interval.
        after_ms: u64,
    },
    /// The second click of a double click: open the main window (and no menu).
    OpenWindow,
}

impl ClickSeries {
    /// A left click (button up) at `now_ms`, with the system's double-click `interval_ms`.
    pub fn click(&mut self, now_ms: u64, interval_ms: u64) -> ClickStep {
        if let Some((at, _)) = self.pending.take()
            && now_ms.saturating_sub(at) <= interval_ms
        {
            return ClickStep::OpenWindow;
        }
        self.generation += 1;
        self.pending = Some((now_ms, self.generation));
        ClickStep::Wait { generation: self.generation, after_ms: interval_ms }
    }

    /// The timer of `generation` fired: `true` when no second click came, so the menu opens. A
    /// timer a double click or a later click made stale answers `false`.
    pub fn expire(&mut self, generation: u64) -> bool {
        match self.pending {
            Some((_, waiting)) if waiting == generation => {
                self.pending = None;
                true
            }
            _ => false,
        }
    }

    /// Something else took the item (a right click opens the menu at once): the click that waits
    /// opens nothing, and it does not pair with the next one.
    pub fn cancel(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZES: [u32; 5] = [16, 20, 24, 32, 36];

    #[test]
    fn regression_a_rebuild_that_read_the_state_earlier_never_installs_over_a_newer_one() {
        // CI 2026-09-29 (run 36601783515, the macOS tray smoke): AI 润色 switched back on, the
        // rebuild the choice forced had read the state before the change and installed its menu
        // after the one the settings event built from the new state.
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::{Arc, Mutex, mpsc};
        use std::time::Duration;
        let menu = Arc::new(MenuSync::new(0u32));
        let state = Arc::new(AtomicU32::new(1));
        let installed = Arc::new(Mutex::new(Vec::new()));
        let (read_tx, read_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        // The forced rebuild reads the state before the change, then is slow to install: it waits
        // for the event's rebuild, or 500 ms when that one is (rightly) held back by the lock.
        let forced = {
            let (menu, state, installed) = (menu.clone(), state.clone(), installed.clone());
            std::thread::spawn(move || {
                menu.sync(
                    true,
                    |_| {
                        let read = state.load(Ordering::SeqCst);
                        read_tx.send(()).unwrap();
                        read
                    },
                    |next| {
                        let _ = go_rx.recv_timeout(Duration::from_millis(500));
                        installed.lock().unwrap().push(*next);
                        Ok::<(), ()>(())
                    },
                )
            })
        };
        read_rx.recv().unwrap();
        state.store(2, Ordering::SeqCst);
        let event = {
            let (menu, state, installed) = (menu.clone(), state.clone(), installed.clone());
            std::thread::spawn(move || {
                menu.sync(
                    false,
                    |_| state.load(Ordering::SeqCst),
                    |next| {
                        installed.lock().unwrap().push(*next);
                        Ok::<(), ()>(())
                    },
                )
            })
        };
        assert_eq!(event.join().unwrap(), Ok(true));
        go_tx.send(()).ok();
        assert_eq!(forced.join().unwrap(), Ok(true));
        assert_eq!(*installed.lock().unwrap(), vec![1, 2], "the menu installed last shows the newest state");
    }

    #[test]
    fn menu_sync_rebuilds_on_change_or_when_forced_and_retries_after_an_error() {
        let menu = MenuSync::new(1u32);
        let mut installs = 0;
        let mut count = |_: &u32| {
            installs += 1;
            Ok::<(), ()>(())
        };
        assert_eq!(menu.sync(false, |_| 1, &mut count), Ok(false), "unchanged: nothing to do");
        assert_eq!(menu.sync(true, |_| 1, &mut count), Ok(true), "forced");
        assert_eq!(menu.sync(false, |shown| shown + 1, &mut count), Ok(true), "changed");
        assert_eq!(installs, 2);
        // A failed install keeps the old model, so the same state is tried again.
        assert_eq!(menu.sync(false, |_| 3, |_| Err("no menu")), Err("no menu"));
        assert_eq!(menu.sync(false, |_| 3, |_| Ok::<(), &str>(())), Ok(true));
        assert_eq!(menu.sync(false, |shown| *shown, |_| Ok::<(), ()>(())), Ok(false));
    }

    fn pixel(buf: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    /// The pixel under a point of the 100-unit image.
    fn at(buf: &[u8], size: u32, u: f64, v: f64) -> [u8; 4] {
        let to_px = |c: f64| ((c / 100.0 * f64::from(size)).floor() as u32).min(size - 1);
        pixel(buf, size, to_px(u), to_px(v))
    }

    /// The middle of a bar or of the cursor.
    fn middle((left, top, width, height): Pill) -> (f64, f64) {
        (left + width / 2.0, top + height / 2.0)
    }

    /// High on the cursor, clear of the badge in the bottom-right corner.
    fn cursor_upper(mark: &Mark) -> (f64, f64) {
        let (left, top, width, height) = mark.cursor;
        (left + width / 2.0, top + height * 0.15)
    }

    /// Between the last bar and the cursor.
    fn before_cursor(mark: &Mark) -> (f64, f64) {
        let (left, _, width, _) = mark.bars[2];
        ((left + width + mark.cursor.0) / 2.0, 50.0)
    }

    /// Between the first two bars.
    fn between_bars(mark: &Mark) -> (f64, f64) {
        let (left, _, width, _) = mark.bars[0];
        ((left + width + mark.bars[1].0) / 2.0, 50.0)
    }

    fn colour_distance(a: [u8; 4], b: [u8; 3]) -> u32 {
        a.iter().zip(b).map(|(&x, y)| u32::from(x.abs_diff(y)).pow(2)).sum()
    }

    /// Opaque and within the tile's gradient, channel by channel.
    fn is_tile(p: [u8; 4]) -> bool {
        p[3] == 255 && (0..3).all(|c| (TILE_FROM[c].min(TILE_TO[c])..=TILE_FROM[c].max(TILE_TO[c])).contains(&p[c]))
    }

    /// The cursor's gradient where `v` is.
    fn cursor_colour(mark: &Mark, v: f64) -> [u8; 3] {
        let (_, top, _, height) = mark.cursor;
        mix(CURSOR_FROM, CURSOR_TO, (v - top) / height)
    }

    /// In pixels: (left, top, width, height).
    fn in_pixels(size: u32, (left, top, width, height): Pill) -> [f64; 4] {
        [left, top, width, height].map(|c| c * f64::from(size) / 100.0)
    }

    #[test]
    fn the_fit_keeps_the_design_at_1024_and_puts_every_edge_on_a_pixel_below() {
        let design = Mark::fitted(1024, 1.0);
        let expected = [(244.0, 402.0, 84.0, 220.0), (376.0, 302.0, 84.0, 420.0), (508.0, 362.0, 84.0, 300.0), (708.0, 232.0, 72.0, 560.0)];
        for (part, want) in design.bars.into_iter().chain([design.cursor]).zip(expected) {
            assert_eq!(in_pixels(1024, part).map(f64::round), [want.0, want.1, want.2, want.3]);
        }
        // The fits scripts/render-icons.py prints (`--fit 16 24 32 48`): the app icon and the tray
        // fit the mark alike.
        let fits: [(u32, [[f64; 4]; 4]); 4] = [
            (16, [[4.0, 6.0, 1.0, 4.0], [6.0, 4.0, 1.0, 8.0], [8.0, 5.0, 1.0, 6.0], [11.0, 3.0, 1.0, 10.0]]),
            (24, [[6.0, 9.0, 2.0, 6.0], [9.0, 7.0, 2.0, 10.0], [12.0, 8.0, 2.0, 8.0], [16.0, 5.0, 2.0, 14.0]]),
            (32, [[7.0, 13.0, 3.0, 6.0], [12.0, 9.0, 3.0, 14.0], [17.0, 11.0, 3.0, 10.0], [23.0, 7.0, 2.0, 18.0]]),
            (48, [[12.0, 19.0, 4.0, 10.0], [18.0, 14.0, 4.0, 20.0], [24.0, 17.0, 4.0, 14.0], [33.0, 11.0, 3.0, 26.0]]),
        ];
        for (size, want) in fits {
            let mark = Mark::fitted(size, 1.0);
            let got: Vec<[f64; 4]> = mark.bars.into_iter().chain([mark.cursor]).map(|part| in_pixels(size, part).map(f64::round)).collect();
            assert_eq!(got, want, "{size}");
        }
        for scale in [1.0, TEMPLATE_SCALE] {
            for size in 16..=64 {
                let mark = Mark::fitted(size, scale);
                let parts: Vec<[f64; 4]> = mark.bars.into_iter().chain([mark.cursor]).map(|part| in_pixels(size, part)).collect();
                for part in &parts {
                    assert!(part.iter().all(|c| (c - c.round()).abs() < 1e-9), "{size} × {scale}: {part:?} is not on whole pixels");
                    // Centred on the image's middle row.
                    assert!((part[1] + part[3] / 2.0 - f64::from(size) / 2.0).abs() < 1e-9, "{size} × {scale}: {part:?}");
                    assert!(part[2] >= 1.0, "{size} × {scale}: {part:?}");
                }
                // Left to right with a gap of at least a pixel, and centred as a whole.
                for pair in parts.windows(2) {
                    assert!(pair[1][0] - (pair[0][0] + pair[0][2]) >= 1.0 - 1e-9, "{size} × {scale}: {pair:?}");
                }
                let right = parts[3][0] + parts[3][2];
                assert!((parts[0][0] + right - f64::from(size)).abs() < 1e-9, "{size} × {scale}: {parts:?}");
                // Short, tall, middle; the cursor tallest; never narrower than a bar's... cursor.
                let heights = parts.iter().map(|p| p[3]).collect::<Vec<_>>();
                assert!(heights[0] < heights[2] && heights[2] < heights[1] && heights[1] < heights[3], "{size} × {scale}: {heights:?}");
                assert!(parts[3][2] <= parts[0][2], "{size} × {scale}: the cursor is no wider than a bar");
            }
        }
    }

    /// Regression (user 2026-10-08, 清晰度要提高下): drawn as designed, a bar one to three pixels
    /// wide had its sides between pixels and smeared into grey columns. Fitted, across the middle
    /// row every bar and the cursor are solid and the pixels beside them are the plain tile
    /// (transparent in the template).
    #[test]
    fn regression_the_bars_and_the_cursor_have_sharp_sides_at_every_size() {
        for size in SIZES.into_iter().chain([18, 22, 28, 40, 48, 64]) {
            for (style, scale) in [(TrayStyle::Color, 1.0), (TrayStyle::Template, TEMPLATE_SCALE)] {
                let mark = Mark::fitted(size, scale);
                let icon = render_tray_icon(TrayGlyph::Idle, size, style);
                for part in mark.bars.into_iter().chain([mark.cursor]) {
                    let [left, top, width, height] = in_pixels(size, part).map(|c| c.round() as u32);
                    let y = top + height / 2;
                    for x in left..left + width {
                        let p = pixel(&icon, size, x, y);
                        assert_eq!(p[3], 255, "{size} {style:?}: ({x}, {y}) {p:?}");
                        if style == TrayStyle::Color && part == mark.cursor {
                            let v = (f64::from(y) + 0.5) * 100.0 / f64::from(size);
                            assert!(colour_distance(p, cursor_colour(&mark, v)) <= 3 * 2 * 2, "{size}: ({x}, {y}) {p:?}");
                        } else if style == TrayStyle::Color {
                            assert_eq!(p[..3], WAVE, "{size}: ({x}, {y})");
                        }
                    }
                    for x in [left - 1, left + width] {
                        let p = pixel(&icon, size, x, y);
                        if style == TrayStyle::Template {
                            assert_eq!(p[3], 0, "{size} template: ({x}, {y}) {p:?}");
                        } else {
                            let (u, v) = ((f64::from(x) + 0.5) * 100.0 / f64::from(size), (f64::from(y) + 0.5) * 100.0 / f64::from(size));
                            let tile = mix(TILE_FROM, TILE_TO, (u + v) / 200.0);
                            assert!(p[3] == 255 && colour_distance(p, tile) <= 3, "{size}: ({x}, {y}) {p:?} is not the tile {tile:?}");
                        }
                    }
                }
            }
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
    /// logo. The colour icon is the mark: the ink tile with its gradient, three white bars, the cyan
    /// cursor, and transparent only outside the rounded corners.
    #[test]
    fn the_colour_icon_is_the_logo() {
        for size in SIZES {
            let mark = Mark::fitted(size, 1.0);
            let idle = render_tray_icon(TrayGlyph::Idle, size, TrayStyle::Color);
            assert_eq!(idle.len(), (size * size * 4) as usize);
            assert!(is_tile(at(&idle, size, 50.0, 10.0)), "{size}: tile");
            // The gradient runs from the top left to the bottom right.
            let (light, dark) = (at(&idle, size, 92.0, 50.0), at(&idle, size, 8.0, 50.0));
            assert!(colour_distance(dark, TILE_FROM) < colour_distance(light, TILE_FROM), "{size}: {dark:?} {light:?}");
            for bar in mark.bars {
                let (u, v) = middle(bar);
                assert_eq!(at(&idle, size, u, v), [255, 255, 255, 255], "{size}: ({u}, {v})");
            }
            let (u, v) = cursor_upper(&mark);
            let cursor = at(&idle, size, u, v);
            assert!(cursor[3] == 255 && colour_distance(cursor, cursor_colour(&mark, v)) <= 3 * 6 * 6, "{size}: {cursor:?}");
            for (u, v) in [before_cursor(&mark), between_bars(&mark)] {
                assert!(is_tile(at(&idle, size, u, v)), "{size}: the gap at ({u}, {v})");
            }
            assert_eq!(pixel(&idle, size, 0, 0)[3], 0, "{size}: the rounded corner is transparent");
            assert_eq!(pixel(&idle, size, size - 1, size - 1)[3], 0, "{size}");
            // The old ring was one colour everywhere; the mark has the tile, white and cyan.
            let opaque: Vec<[u8; 4]> = idle.as_chunks::<4>().0.iter().copied().filter(|p| p[3] == 255).collect();
            assert!(opaque.iter().any(|&p| is_tile(p)), "{size}: no tile");
            assert!(opaque.iter().any(|&p| p[..3] == WAVE), "{size}: no white bar");
            assert!(opaque.iter().any(|&p| colour_distance(p, CURSOR_TO) < 40 * 40), "{size}: no cyan cursor");
            assert!(!opaque.iter().any(|&p| p[..3] == PROCESSING_RGB), "{size}: no accent-blue ring any more");
        }
    }

    #[test]
    fn the_colour_badge_shows_the_phase_in_the_corner() {
        for size in SIZES {
            let mark = Mark::fitted(size, 1.0);
            let idle = render_tray_icon(TrayGlyph::Idle, size, TrayStyle::Color);
            let listening = render_tray_icon(TrayGlyph::Listening, size, TrayStyle::Color);
            let processing = render_tray_icon(TrayGlyph::Processing, size, TrayStyle::Color);
            let (cu, cv) = BADGE_CENTRE;
            // Idle, the corner shows the mark (the tile or the cursor's cyan end), no badge colour.
            let corner = at(&idle, size, cu, cv);
            assert_eq!(corner[3], 255, "{size}");
            assert!(colour_distance(corner, LISTENING_RGB) > 40 * 40 && colour_distance(corner, PROCESSING_RGB) > 40 * 40, "{size}: {corner:?}");
            assert_eq!(at(&listening, size, cu, cv), [LISTENING_RGB[0], LISTENING_RGB[1], LISTENING_RGB[2], 255], "{size}");
            assert_eq!(at(&processing, size, cu, cv), [PROCESSING_RGB[0], PROCESSING_RGB[1], PROCESSING_RGB[2], 255], "{size}");
            // The white gap separates the badge from the tile (sampled halfway between the badge
            // edge and the corner arc, above the centre; below 32 px no whole pixel fits in it).
            if size >= 32 {
                assert_eq!(at(&listening, size, cu, cv - (BADGE_RADIUS + BADGE_GAP_RADIUS) / 2.0), [255, 255, 255, 255], "{size}");
            }
            // Away from the corner nothing changes.
            for (u, v) in [(50.0, 10.0), middle(mark.bars[0]), middle(mark.bars[1]), middle(mark.bars[2]), cursor_upper(&mark), (10.0, 90.0)] {
                assert_eq!(at(&listening, size, u, v), at(&idle, size, u, v), "{size}: ({u}, {v})");
            }
        }
    }

    #[test]
    fn the_template_is_the_bars_and_the_cursor_alone_in_ink() {
        for size in SIZES {
            let mark = Mark::fitted(size, TEMPLATE_SCALE);
            let idle = render_tray_icon(TrayGlyph::Idle, size, TrayStyle::Template);
            // Only alpha counts in a template: every pixel is ink or transparent.
            assert!(idle.as_chunks::<4>().0.iter().all(|p| p[..3] == INK || p[3] == 0), "{size}");
            for (u, v) in [middle(mark.bars[0]), middle(mark.bars[1]), middle(mark.bars[2]), cursor_upper(&mark)] {
                assert_eq!(at(&idle, size, u, v)[3], 255, "{size}: ink at ({u}, {v})");
            }
            // No tile: the background above, below and around the mark is clear, and so are the gaps.
            assert_eq!(at(&idle, size, 50.0, 3.0)[3], 0, "{size}");
            assert_eq!(at(&idle, size, 3.0, 97.0)[3], 0, "{size}");
            for (u, v) in [between_bars(&mark), before_cursor(&mark)] {
                assert_eq!(at(&idle, size, u, v)[3], 0, "{size}: the gap at ({u}, {v})");
            }
            // Scaled up: the cursor reaches close to the top edge and the bottom one.
            let rows_with_ink: Vec<u32> = (0..size).filter(|&y| (0..size).any(|x| pixel(&idle, size, x, y)[3] > 0)).collect();
            let (top, bottom) = (rows_with_ink[0], rows_with_ink[rows_with_ink.len() - 1]);
            assert!(f64::from(bottom - top + 1) >= 0.8 * f64::from(size), "{size}: the mark spans {top}..={bottom}");
            let listening = render_tray_icon(TrayGlyph::Listening, size, TrayStyle::Template);
            let processing = render_tray_icon(TrayGlyph::Processing, size, TrayStyle::Template);
            let (cu, cv) = TEMPLATE_BADGE_CENTRE;
            assert_eq!(at(&listening, size, cu, cv)[3], 255, "{size}: filled badge");
            if size >= 32 {
                // The badge sits over the cursor's lower end, its gap cut out of the cursor.
                let (gu, gv) = (cursor_upper(&mark).0, cv - 19.0);
                assert_eq!(at(&idle, size, gu, gv)[3], 255, "{size}: the cursor at ({gu}, {gv})");
                assert_eq!(at(&listening, size, gu, gv)[3], 0, "{size}: the gap at ({gu}, {gv})");
                assert_eq!(at(&processing, size, cu, cv)[3], 0, "{size}: the ring's hole");
                let ring = (TEMPLATE_BADGE_RADIUS + TEMPLATE_BADGE_HOLE_RADIUS) / 2.0;
                assert_eq!(at(&processing, size, cu, cv - ring)[3], 255, "{size}: the ring");
            }
        }
    }

    #[test]
    fn edges_are_anti_aliased() {
        let icon = render_tray_icon(TrayGlyph::Idle, 32, TrayStyle::Template);
        assert!(icon.as_chunks::<4>().0.iter().any(|p| p[3] > 0 && p[3] < 255), "partial coverage on the round ends");
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

    #[test]
    fn a_click_alone_opens_the_menu_once_the_double_click_interval_has_passed() {
        let mut clicks = ClickSeries::default();
        let ClickStep::Wait { generation, after_ms } = clicks.click(1_000, 500) else { panic!("a first click waits") };
        assert_eq!(after_ms, 500);
        assert!(clicks.expire(generation), "no second click: the menu opens");
        assert!(!clicks.expire(generation), "a timer fires once");
    }

    #[test]
    fn a_second_click_within_the_interval_opens_the_window_and_the_first_timer_opens_nothing() {
        let mut clicks = ClickSeries::default();
        let ClickStep::Wait { generation, .. } = clicks.click(1_000, 500) else { panic!("waits") };
        assert_eq!(clicks.click(1_300, 500), ClickStep::OpenWindow);
        assert!(!clicks.expire(generation), "the double click took the wait: no menu");
        // Exactly the interval still counts as a double click.
        let ClickStep::Wait { .. } = clicks.click(5_000, 500) else { panic!("waits") };
        assert_eq!(clicks.click(5_500, 500), ClickStep::OpenWindow);
    }

    #[test]
    fn a_slow_second_click_starts_a_series_of_its_own_and_a_third_click_waits_again() {
        let mut clicks = ClickSeries::default();
        let ClickStep::Wait { generation: first, .. } = clicks.click(1_000, 500) else { panic!("waits") };
        // The timer ran late; the second click came after the interval: not a double click.
        let ClickStep::Wait { generation: second, .. } = clicks.click(1_600, 500) else { panic!("waits") };
        assert_ne!(first, second);
        assert!(!clicks.expire(first), "the stale timer opens nothing");
        assert!(clicks.expire(second), "the newest click's menu opens");
        // Click, click (window), click: the third click starts a new wait.
        let ClickStep::Wait { .. } = clicks.click(9_000, 500) else { panic!("waits") };
        assert_eq!(clicks.click(9_100, 500), ClickStep::OpenWindow);
        assert!(matches!(clicks.click(9_200, 500), ClickStep::Wait { .. }));
    }

    #[test]
    fn a_right_click_takes_the_waiting_click_so_its_timer_opens_no_second_menu() {
        let mut clicks = ClickSeries::default();
        let ClickStep::Wait { generation, .. } = clicks.click(1_000, 500) else { panic!("waits") };
        clicks.cancel();
        assert!(!clicks.expire(generation), "the right click's menu is the only one");
        assert!(matches!(clicks.click(1_200, 500), ClickStep::Wait { .. }), "and the next click starts afresh");
    }
}
