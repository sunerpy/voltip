//! Linux: what the desktop shell sets before GTK and WebKitGTK open the first window
//! (docs/dictation.md §14.8, issue #228).
//!
//! On a Wayland session with an NVIDIA GPU, WebKitGTK's DMA-BUF renderer can end the app a second
//! after it starts: the compositor refuses a frame on a surface that already carries an explicit
//! synchronisation object (`wp_linux_drm_syncobj_surface_v1`, "Explicit Sync only supported on
//! dmabuf buffers"), and GTK quits with `Error 71 (Protocol error) dispatching to Wayland
//! display`. Turning that renderer off ([`WEBKIT_DISABLE_DMABUF_RENDERER`]) avoids it, at some cost
//! in drawing speed, so the shell does it only where it is needed: GTK opens its display on
//! Wayland and a GPU uses an NVIDIA driver, NVIDIA's own (`nvidia`) or the kernel's open one
//! (`nouveau`, under Mesa's NVK). A value the user set is never replaced.

use std::ffi::{OsStr, OsString};

/// The variable WebKitGTK reads to leave its DMA-BUF renderer off.
pub const WEBKIT_DISABLE_DMABUF_RENDERER: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

/// An NVIDIA kernel driver, by the name the kernel gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NvidiaDriver {
    /// NVIDIA's own driver (`nvidia`): the proprietary and the open kernel modules alike.
    Nvidia,
    /// The kernel's open driver (`nouveau`), which Mesa's NVK runs on.
    Nouveau,
}

impl NvidiaDriver {
    /// The NVIDIA driver a kernel driver name stands for (the name a GPU's
    /// `/sys/class/drm/card<N>/device/driver` link ends in); `None` for every other driver.
    pub fn from_kernel_name(name: &str) -> Option<Self> {
        match name {
            "nvidia" => Some(Self::Nvidia),
            "nouveau" => Some(Self::Nouveau),
            _ => None,
        }
    }

    /// The kernel's name for the driver.
    pub const fn kernel_name(self) -> &'static str {
        match self {
            Self::Nvidia => "nvidia",
            Self::Nouveau => "nouveau",
        }
    }
}

/// WebKitGTK's DMA-BUF renderer, as the shell leaves it for the first window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmabufRenderer {
    /// Nothing changes: GTK does not open its display on Wayland, or no GPU uses an NVIDIA driver.
    Keep,
    /// Turned off: GTK opens its display on Wayland and a GPU uses this NVIDIA driver.
    Disable(NvidiaDriver),
    /// The same session and GPU, but the user set [`WEBKIT_DISABLE_DMABUF_RENDERER`] already, to
    /// whatever value: theirs stays.
    UserChoice(NvidiaDriver),
}

impl DmabufRenderer {
    /// The variables the shell sets for this choice, as name and value.
    pub const fn variables(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Disable(_) => &[(WEBKIT_DISABLE_DMABUF_RENDERER, "1")],
            Self::Keep | Self::UserChoice(_) => &[],
        }
    }
}

/// Whether GTK 3 opens its display on Wayland. `WAYLAND_DISPLAY` must name a socket, and
/// `GDK_BACKEND`, when set, must try Wayland first: GTK splits it at commas and goes through the
/// entries in order, so its first entry must be `wayland`, or `*` (every backend, in GTK's own
/// order, which starts with Wayland). GTK compares the entries exactly, letter case included.
pub fn gtk_opens_wayland(wayland_display: Option<&OsStr>, gdk_backend: Option<&OsStr>) -> bool {
    if wayland_display.is_none_or(OsStr::is_empty) {
        return false;
    }
    match gdk_backend {
        None => true,
        Some(list) => matches!(list.to_string_lossy().split(',').next(), Some("wayland" | "*")),
    }
}

/// WebKitGTK's DMA-BUF renderer for this environment and these GPU drivers (#228): turned off when
/// GTK opens its display on Wayland ([`gtk_opens_wayland`]) and a GPU uses an NVIDIA driver
/// ([`NvidiaDriver::from_kernel_name`]; the first such driver is the one named), unless the user
/// set [`WEBKIT_DISABLE_DMABUF_RENDERER`] already. `env` reads one environment variable; `drivers`
/// are the kernel driver names of the machine's GPUs.
pub fn dmabuf_renderer<S: AsRef<str>>(env: impl Fn(&str) -> Option<OsString>, drivers: impl IntoIterator<Item = S>) -> DmabufRenderer {
    if !gtk_opens_wayland(env("WAYLAND_DISPLAY").as_deref(), env("GDK_BACKEND").as_deref()) {
        return DmabufRenderer::Keep;
    }
    let Some(driver) = drivers.into_iter().find_map(|name| NvidiaDriver::from_kernel_name(name.as_ref())) else {
        return DmabufRenderer::Keep;
    };
    if env(WEBKIT_DISABLE_DMABUF_RENDERER).is_some() { DmabufRenderer::UserChoice(driver) } else { DmabufRenderer::Disable(driver) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An environment of `vars`: every other variable is unset.
    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| vars.iter().find(|(key, _)| *key == name).map(|(_, value)| OsString::from(value))
    }

    /// The session of issue #228: GNOME Shell 50.1 on Wayland, launched from the desktop, without
    /// `GDK_BACKEND`.
    const GNOME_WAYLAND: &[(&str, &str)] = &[("WAYLAND_DISPLAY", "wayland-0"), ("XDG_SESSION_TYPE", "wayland"), ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")];

    const DISABLED: &[(&str, &str)] = &[(WEBKIT_DISABLE_DMABUF_RENDERER, "1")];

    /// Regression (#228): Ubuntu 26.04, GNOME Wayland, an RTX 3060 on nouveau (Mesa NVK) next to an
    /// Intel UHD 770 on i915. 0.2.1 set nothing, WebKitGTK's DMA-BUF renderer ran, and the app
    /// quit with Error 71 a second after it started.
    #[test]
    fn regression_gnome_wayland_with_nouveau_and_i915_turns_the_dmabuf_renderer_off() {
        let choice = dmabuf_renderer(env(GNOME_WAYLAND), ["nouveau", "i915"]);
        assert_eq!(choice, DmabufRenderer::Disable(NvidiaDriver::Nouveau));
        assert_eq!(choice.variables(), DISABLED);
        // The card order does not matter.
        assert_eq!(dmabuf_renderer(env(GNOME_WAYLAND), ["i915", "nouveau"]).variables(), DISABLED);
    }

    /// NVIDIA's own driver, alone or next to an integrated GPU, calls for the same.
    #[test]
    fn the_proprietary_driver_turns_it_off_too() {
        for drivers in [&["nvidia"][..], &["i915", "nvidia"], &["amdgpu", "nvidia"]] {
            let choice = dmabuf_renderer(env(GNOME_WAYLAND), drivers);
            assert_eq!(choice, DmabufRenderer::Disable(NvidiaDriver::Nvidia), "{drivers:?}");
            assert_eq!(choice.variables(), DISABLED, "{drivers:?}");
        }
        // Two NVIDIA drivers on one machine: the first one found is named.
        assert_eq!(dmabuf_renderer(env(GNOME_WAYLAND), ["nouveau", "nvidia"]), DmabufRenderer::Disable(NvidiaDriver::Nouveau));
    }

    /// Without an NVIDIA GPU nothing changes, on Wayland too: Intel, AMD, a virtual machine's GPU,
    /// the firmware's framebuffer, or no GPU found at all.
    #[test]
    fn other_gpus_keep_the_renderer() {
        let others: [&[&str]; 9] =
            [&["i915"], &["xe"], &["amdgpu"], &["radeon"], &["i915", "amdgpu"], &["virtio_gpu"], &["simple-framebuffer"], &["vmwgfx"], &[]];
        for drivers in others {
            let choice = dmabuf_renderer(env(GNOME_WAYLAND), drivers);
            assert_eq!(choice, DmabufRenderer::Keep, "{drivers:?}");
            assert!(choice.variables().is_empty(), "{drivers:?}");
        }
    }

    /// An X11 session keeps the renderer, NVIDIA GPU or not: no `WAYLAND_DISPLAY`, or an empty one.
    #[test]
    fn x11_sessions_keep_the_renderer() {
        let x11: [&[(&str, &str)]; 3] =
            [&[("DISPLAY", ":0"), ("XDG_SESSION_TYPE", "x11")], &[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", ""), ("XDG_SESSION_TYPE", "x11")], &[]];
        for vars in x11 {
            for drivers in [&["nouveau", "i915"][..], &["nvidia"]] {
                let choice = dmabuf_renderer(env(vars), drivers);
                assert_eq!(choice, DmabufRenderer::Keep, "{vars:?} {drivers:?}");
                assert!(choice.variables().is_empty(), "{vars:?} {drivers:?}");
            }
        }
    }

    /// `GDK_BACKEND` decides which display GTK opens: its first entry, compared exactly.
    #[test]
    fn gdk_backend_decides_whether_gtk_opens_wayland() {
        let cases: [(Option<&str>, bool); 13] = [
            (None, true),
            (Some("wayland"), true),
            (Some("wayland,x11"), true),
            (Some("wayland,x11,*"), true),
            // Every backend in GTK's order, which tries Wayland first.
            (Some("*"), true),
            (Some("x11"), false),
            (Some("x11,wayland"), false),
            (Some("x11,*"), false),
            (Some("broadway"), false),
            // GTK opens no display at all with these.
            (Some(""), false),
            (Some("Wayland"), false),
            (Some(" wayland"), false),
            (Some("waylandx"), false),
        ];
        for (backend, wayland) in cases {
            assert_eq!(gtk_opens_wayland(Some(OsStr::new("wayland-0")), backend.map(OsStr::new)), wayland, "GDK_BACKEND={backend:?}");
            // Without a Wayland display, GTK opens none whatever GDK_BACKEND says.
            assert!(!gtk_opens_wayland(None, backend.map(OsStr::new)), "no WAYLAND_DISPLAY, GDK_BACKEND={backend:?}");
            assert!(!gtk_opens_wayland(Some(OsStr::new("")), backend.map(OsStr::new)), "empty WAYLAND_DISPLAY, GDK_BACKEND={backend:?}");
        }
    }

    /// `GDK_BACKEND=x11` on a Wayland session, the workaround of #228: GTK goes through XWayland,
    /// and the renderer stays as it is.
    #[test]
    fn gdk_backend_x11_keeps_the_renderer() {
        let vars = [GNOME_WAYLAND, &[("GDK_BACKEND", "x11")]].concat();
        assert_eq!(dmabuf_renderer(env(&vars), ["nouveau", "i915"]), DmabufRenderer::Keep);
        let vars = [GNOME_WAYLAND, &[("GDK_BACKEND", "wayland,x11")]].concat();
        assert_eq!(dmabuf_renderer(env(&vars), ["nouveau", "i915"]), DmabufRenderer::Disable(NvidiaDriver::Nouveau));
    }

    /// A value the user set stays, whatever it is, `0` included: WebKitGTK leaves its DMA-BUF
    /// renderer on for `0` and turns it off for any other value, the empty one too.
    #[test]
    fn a_value_the_user_set_is_kept() {
        for value in ["1", "0", "", "false", "yes"] {
            let vars = [GNOME_WAYLAND, &[(WEBKIT_DISABLE_DMABUF_RENDERER, value)]].concat();
            let choice = dmabuf_renderer(env(&vars), ["nouveau", "i915"]);
            assert_eq!(choice, DmabufRenderer::UserChoice(NvidiaDriver::Nouveau), "{value:?}");
            assert!(choice.variables().is_empty(), "{value:?}");
            // Where nothing would change anyway, it is simply kept.
            assert_eq!(dmabuf_renderer(env(&vars), ["i915"]), DmabufRenderer::Keep, "{value:?}");
        }
    }

    /// The two NVIDIA drivers by the names the kernel gives them, and nothing else.
    #[test]
    fn nvidia_drivers_are_named_as_the_kernel_names_them() {
        for driver in [NvidiaDriver::Nvidia, NvidiaDriver::Nouveau] {
            assert_eq!(NvidiaDriver::from_kernel_name(driver.kernel_name()), Some(driver));
        }
        for other in ["i915", "xe", "amdgpu", "radeon", "NVIDIA", "Nouveau", "nvidia-drm", "nvidia_drm", "", "nouveau "] {
            assert_eq!(NvidiaDriver::from_kernel_name(other), None, "{other:?}");
        }
    }
}
