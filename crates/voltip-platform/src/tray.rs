//! The tray icon's three states, drawn to RGBA at runtime so no per-state asset has to be shipped
//! or kept in sync (docs/dictation.md §15.4). On macOS the shell marks the image as a *template*
//! and the menu bar tints it for the light / dark theme; on Windows it is drawn in the accent
//! colour so it reads on both taskbar themes.

/// What the tray shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrayGlyph {
    /// A ring: ready, nothing in flight (also the `done` / `failed` / `cancelled` dwell).
    Idle,
    /// A filled disc: the microphone is open.
    Listening,
    /// A ring with a centre dot: transcribing / refining / inserting.
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

/// Icon side in pixels: macOS menu bar icons are 22 pt, drawn at 2× for Retina; Windows scales
/// down from this without visible artefacts.
pub const TRAY_ICON_SIZE: u32 = 44;
/// The colour used where the platform does not tint template images (Windows): the app accent.
pub const TRAY_ACCENT_RGB: [u8; 3] = [0x2F, 0x6F, 0xED];
/// The colour for template images (macOS ignores it and keeps only alpha).
pub const TRAY_TEMPLATE_RGB: [u8; 3] = [0, 0, 0];

/// Render `glyph` as a `size × size` RGBA buffer (row-major, top to bottom) in `rgb`, fully opaque
/// where drawn and transparent elsewhere (a 1 px anti-aliased edge).
pub fn render_glyph(glyph: TrayGlyph, size: u32, rgb: [u8; 3]) -> Vec<u8> {
    let s = f64::from(size);
    let centre = (s - 1.0) / 2.0;
    let outer = s * 0.42;
    let inner = s * 0.28;
    let dot = s * 0.14;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let dx = f64::from(x) - centre;
            let dy = f64::from(y) - centre;
            let r = (dx * dx + dy * dy).sqrt();
            let coverage = match glyph {
                TrayGlyph::Listening => disc(r, outer),
                TrayGlyph::Idle => disc(r, outer) - disc(r, inner),
                TrayGlyph::Processing => disc(r, outer) - disc(r, inner) + disc(r, dot),
            };
            let alpha = (coverage.clamp(0.0, 1.0) * 255.0).round();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let a = alpha as u8;
            let i = ((y * size + x) * 4) as usize;
            out[i..i + 3].copy_from_slice(&rgb);
            out[i + 3] = a;
        }
    }
    out
}

/// Coverage of a pixel at distance `r` from the centre by a disc of radius `radius`: 1 inside,
/// 0 outside, a linear ramp over the last pixel.
fn disc(r: f64, radius: f64) -> f64 {
    (radius + 0.5 - r).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(buf: &[u8], size: u32, x: u32, y: u32) -> u8 {
        buf[(((y * size + x) * 4) + 3) as usize]
    }

    fn opaque_pixels(buf: &[u8]) -> usize {
        buf.as_chunks::<4>().0.iter().filter(|p| p[3] == 255).count()
    }

    #[test]
    fn glyphs_follow_the_phase_name() {
        assert_eq!(TrayGlyph::from_phase_name("listening"), TrayGlyph::Listening);
        assert_eq!(TrayGlyph::from_phase_name("processing"), TrayGlyph::Processing);
        for idle in ["idle", "done", "failed", "cancelled", "", "something_new"] {
            assert_eq!(TrayGlyph::from_phase_name(idle), TrayGlyph::Idle, "{idle}");
        }
    }

    #[test]
    fn rendered_glyphs_differ_in_the_centre_and_share_the_outline() {
        let size = TRAY_ICON_SIZE;
        let idle = render_glyph(TrayGlyph::Idle, size, TRAY_TEMPLATE_RGB);
        let listening = render_glyph(TrayGlyph::Listening, size, TRAY_TEMPLATE_RGB);
        let processing = render_glyph(TrayGlyph::Processing, size, TRAY_ACCENT_RGB);
        assert_eq!(idle.len(), (size * size * 4) as usize);
        let c = size / 2;
        // Centre: hollow ring, filled disc, centre dot.
        assert_eq!(alpha_at(&idle, size, c, c), 0);
        assert_eq!(alpha_at(&listening, size, c, c), 255);
        assert_eq!(alpha_at(&processing, size, c, c), 255);
        // The ring band (between inner and outer radius) is drawn in all three.
        let band = c + (size as f64 * 0.35) as u32;
        assert_eq!(alpha_at(&idle, size, band, c), 255);
        assert_eq!(alpha_at(&listening, size, band, c), 255);
        assert_eq!(alpha_at(&processing, size, band, c), 255);
        // Corners stay transparent, so the icon has no square backdrop.
        for (x, y) in [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)] {
            assert_eq!(alpha_at(&idle, size, x, y), 0);
            assert_eq!(alpha_at(&listening, size, x, y), 0);
        }
        // Filled > processing > ring in ink.
        assert!(opaque_pixels(&listening) > opaque_pixels(&processing));
        assert!(opaque_pixels(&processing) > opaque_pixels(&idle));
        // Colour channels carry the requested RGB everywhere (template images only use alpha).
        assert!(processing.as_chunks::<4>().0.iter().all(|p| p[..3] == TRAY_ACCENT_RGB));
        assert!(idle.as_chunks::<4>().0.iter().all(|p| p[..3] == TRAY_TEMPLATE_RGB));
    }

    #[test]
    fn tiny_sizes_do_not_panic() {
        for size in [1, 2, 3, 16, 22] {
            let buf = render_glyph(TrayGlyph::Processing, size, TRAY_ACCENT_RGB);
            assert_eq!(buf.len(), (size * size * 4) as usize);
        }
    }
}
