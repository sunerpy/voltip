//! The graphics environment GTK and WebKitGTK start with on Linux (docs/dictation.md §14.8,
//! issue #228). `run` calls [`prepare`] as its first statement: on a Wayland session with an NVIDIA
//! GPU it sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` before the first window, unless the user set the
//! variable already. The decision is `voltip_platform::linux::dmabuf_renderer`; this module reads
//! the GPU drivers, changes the environment, and [`Startup::log`] writes the line once tracing is
//! up.
//!
//! `std::env::set_var` is only sound while no other thread can read or write the environment, so
//! [`prepare`] counts the process's threads first and sets nothing when it is not alone.
// The one `unsafe` block is `set_var`, behind that check.
#![allow(unsafe_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use voltip_platform::linux::{DmabufRenderer, NvidiaDriver, WEBKIT_DISABLE_DMABUF_RENDERER, dmabuf_renderer, gtk_opens_wayland};

/// Debug-only override of the directory the GPU facts are read under (`sys/class/drm`,
/// `proc/driver/nvidia`), `/` otherwise: `tests/cli.rs` and the Wayland smoke
/// (`VOLTIP_SMOKE_GPU`) take the NVIDIA path on machines without one. Release builds ignore it.
pub const DEV_GPU_ROOT_ENV: &str = "VOLTIP_DEV_GPU_ROOT";

/// The directory the GPU facts are read under: `/`, or `value` (`VOLTIP_DEV_GPU_ROOT`) in a
/// debug build when it is not empty.
pub fn gpu_root(value: Option<OsString>, debug_build: bool) -> PathBuf {
    match value {
        Some(path) if debug_build && !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from("/"),
    }
}

/// The kernel drivers of the machine's GPUs, read under `root` (`/` outside the tests): the last
/// component of each `sys/class/drm/card<N>/device/driver` link, in card order (connectors such as
/// `card0-DP-1` and render nodes are not cards), then `nvidia` when `proc/driver/nvidia` exists and
/// no card named it (NVIDIA's module loaded without its DRM device). Whatever cannot be read counts
/// as no GPU; reading a link does not wait on the device.
pub fn gpu_drivers(root: &Path) -> Vec<String> {
    let mut cards: Vec<(u64, String)> = std::fs::read_dir(root.join("sys/class/drm"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let number = name.to_str()?.strip_prefix("card").filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))?.parse().ok()?;
            let link = std::fs::read_link(entry.path().join("device/driver")).ok()?;
            Some((number, link.file_name()?.to_str()?.to_owned()))
        })
        .collect();
    cards.sort();
    let mut drivers: Vec<String> = cards.into_iter().map(|(_, driver)| driver).collect();
    let nvidia = NvidiaDriver::Nvidia.kernel_name();
    if root.join("proc/driver/nvidia").exists() && !drivers.iter().any(|driver| driver == nvidia) {
        drivers.push(nvidia.to_owned());
    }
    drivers
}

/// What happened to the environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing to set: the renderer stays as it is, or the user chose.
    Unchanged,
    /// The variables of the decision are set.
    Set,
    /// The decision called for them, but other threads were running (their number, `None` when it
    /// could not be read): nothing was set.
    NotAlone(Option<usize>),
}

/// What [`prepare`] found and did, for [`Startup::log`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Startup {
    /// The GPU drivers found ([`gpu_drivers`]).
    pub drivers: Vec<String>,
    /// Whether GTK opens its display on Wayland (`voltip_platform::linux::gtk_opens_wayland`).
    pub wayland: bool,
    /// The decision.
    pub renderer: DmabufRenderer,
    /// `WEBKIT_DISABLE_DMABUF_RENDERER` as it was before.
    pub user_value: Option<OsString>,
    /// What happened to the environment.
    pub outcome: Outcome,
}

/// `run`'s first statement: the GPU facts under `/` (or [`DEV_GPU_ROOT_ENV`] in a debug build),
/// the decision for this process's environment, and the variables set when it calls for them.
pub fn prepare() -> Startup {
    prepare_in(&gpu_root(std::env::var_os(DEV_GPU_ROOT_ENV), cfg!(debug_assertions)), |name| std::env::var_os(name))
}

/// [`prepare`] with the GPU facts read under `root` and the environment read through `env`. The
/// variables still go into this process's environment, and only while it runs no other thread.
pub fn prepare_in(root: &Path, env: impl Fn(&str) -> Option<OsString>) -> Startup {
    let drivers = gpu_drivers(root);
    let wayland = gtk_opens_wayland(env("WAYLAND_DISPLAY").as_deref(), env("GDK_BACKEND").as_deref());
    let renderer = dmabuf_renderer(&env, &drivers);
    let variables = renderer.variables();
    let outcome = if variables.is_empty() {
        Outcome::Unchanged
    } else {
        match thread_count() {
            Some(1) => {
                for (name, value) in variables {
                    // SAFETY: `/proc/self/task` lists one thread, the one running this: no other
                    // thread exists to read or write the environment while `set_var` changes it,
                    // and none can start meanwhile, since only a running thread of this process
                    // could start one.
                    unsafe { std::env::set_var(name, value) };
                }
                Outcome::Set
            }
            threads => Outcome::NotAlone(threads),
        }
    };
    Startup { drivers, wayland, renderer, user_value: env(WEBKIT_DISABLE_DMABUF_RENDERER), outcome }
}

/// The threads this process runs, one `/proc/self/task` entry each (an entry that cannot be read
/// counts as one); `None` when the directory cannot be read.
fn thread_count() -> Option<usize> {
    std::fs::read_dir("/proc/self/task").ok().map(Iterator::count)
}

impl Startup {
    /// The log line for what [`prepare`] did: the drivers found, never anything about the machine
    /// or the user beyond the variable's own value.
    pub fn log(&self) {
        let gpus = &self.drivers;
        match (self.renderer, self.outcome) {
            (DmabufRenderer::Disable(driver), Outcome::Set) => tracing::info!(
                driver = %driver.kernel_name(),
                ?gpus,
                "WebKitGTK's DMA-BUF renderer off for a Wayland session with an NVIDIA GPU: WEBKIT_DISABLE_DMABUF_RENDERER=1 (#228)"
            ),
            (DmabufRenderer::Disable(driver), Outcome::NotAlone(threads)) => tracing::warn!(
                driver = %driver.kernel_name(),
                ?gpus,
                ?threads,
                "a Wayland session with an NVIDIA GPU, but other threads run already: WEBKIT_DISABLE_DMABUF_RENDERER is not set, and the window may close as it opens (#228)"
            ),
            (DmabufRenderer::UserChoice(driver), _) => tracing::info!(
                driver = %driver.kernel_name(),
                ?gpus,
                value = ?self.user_value.as_deref().unwrap_or_default(),
                "a Wayland session with an NVIDIA GPU: WEBKIT_DISABLE_DMABUF_RENDERER stays as the user set it"
            ),
            _ => tracing::debug!(wayland = self.wayland, ?gpus, "WebKitGTK's DMA-BUF renderer left as it is"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A `sys/class/drm` tree under `root` as the kernel lays it out: `card<N>/device/driver` links
    /// to `../../../bus/pci/drivers/<driver>` (dangling here; only the link is read), a connector
    /// next to each card, the render node of the first one, and the `version` file.
    fn drm(root: &Path, drivers: &[&str]) {
        let drm = root.join("sys/class/drm");
        for (n, driver) in drivers.iter().enumerate() {
            let device = drm.join(format!("card{n}/device"));
            std::fs::create_dir_all(&device).unwrap();
            symlink(format!("../../../bus/pci/drivers/{driver}"), device.join("driver")).unwrap();
            std::fs::create_dir_all(drm.join(format!("card{n}-DP-{}", n + 1))).unwrap();
        }
        if let Some(first) = drivers.first() {
            let render = drm.join("renderD128/device");
            std::fs::create_dir_all(&render).unwrap();
            symlink(format!("../../../bus/pci/drivers/{first}"), render.join("driver")).unwrap();
        }
        std::fs::create_dir_all(&drm).unwrap();
        std::fs::write(drm.join("version"), "drm 1.1.0 20060810\n").unwrap();
    }

    /// An environment of `vars`: every other variable is unset.
    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| vars.iter().find(|(key, _)| *key == name).map(|(_, value)| OsString::from(value))
    }

    /// A writer into a shared buffer, for [`logged`].
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// What [`Startup::log`] writes, at every level, as the shell's subscriber formats it.
    fn logged(startup: &Startup) -> String {
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::fmt().with_max_level(tracing::Level::TRACE).with_ansi(false).with_writer(move || Capture(writer.clone())).finish();
        tracing::subscriber::with_default(subscriber, || startup.log());
        String::from_utf8(buffer.lock().unwrap().clone()).unwrap()
    }

    /// The session of issue #228: GNOME Wayland, started from the desktop, no `GDK_BACKEND`.
    const GNOME_WAYLAND: &[(&str, &str)] = &[("WAYLAND_DISPLAY", "wayland-0"), ("XDG_SESSION_TYPE", "wayland"), ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")];

    /// The reporter's two GPUs (#228), each card's driver from its link, in card order; the
    /// connectors, the render node and the `version` file are not cards.
    #[test]
    fn each_card_s_driver_comes_from_its_device_link() {
        let root = tempfile::tempdir().unwrap();
        drm(root.path(), &["nouveau", "i915"]);
        assert_eq!(gpu_drivers(root.path()), ["nouveau", "i915"]);
        // Card numbers, not names, give the order: card10 comes after card2.
        let root = tempfile::tempdir().unwrap();
        drm(root.path(), &["amdgpu", "i915", "xe", "amdgpu", "amdgpu", "amdgpu", "amdgpu", "amdgpu", "amdgpu", "amdgpu", "nouveau"]);
        assert_eq!(gpu_drivers(root.path()).last().map(String::as_str), Some("nouveau"));
        assert_eq!(gpu_drivers(root.path())[2], "xe");
    }

    /// NVIDIA's module without its DRM device shows only under `/proc/driver/nvidia`: it counts,
    /// once.
    #[test]
    fn the_nvidia_module_counts_without_a_drm_card() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("proc/driver/nvidia/gpus")).unwrap();
        assert_eq!(gpu_drivers(root.path()), ["nvidia"]);
        drm(root.path(), &["i915"]);
        assert_eq!(gpu_drivers(root.path()), ["i915", "nvidia"]);
        let root = tempfile::tempdir().unwrap();
        drm(root.path(), &["nvidia"]);
        std::fs::create_dir_all(root.path().join("proc/driver/nvidia")).unwrap();
        assert_eq!(gpu_drivers(root.path()), ["nvidia"], "named by a card already");
    }

    /// What cannot be read counts as no GPU: no `sys/class/drm` (a container), a card without a
    /// driver link, a driver that is a file rather than a link, a `drm` that is a file.
    #[test]
    fn unreadable_facts_count_as_no_gpu() {
        let root = tempfile::tempdir().unwrap();
        assert!(gpu_drivers(root.path()).is_empty());
        assert!(gpu_drivers(&root.path().join("missing")).is_empty());
        let drm_dir = root.path().join("sys/class/drm");
        std::fs::create_dir_all(drm_dir.join("card0/device")).unwrap();
        std::fs::create_dir_all(drm_dir.join("card1/device")).unwrap();
        std::fs::write(drm_dir.join("card1/device/driver"), "nouveau").unwrap();
        std::fs::create_dir_all(drm_dir.join("cardX/device")).unwrap();
        symlink("../../../bus/pci/drivers/nouveau", drm_dir.join("cardX/device/driver")).unwrap();
        assert!(gpu_drivers(root.path()).is_empty(), "{:?}", gpu_drivers(root.path()));
        let file = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(file.path().join("sys/class")).unwrap();
        std::fs::write(file.path().join("sys/class/drm"), "").unwrap();
        assert!(gpu_drivers(file.path()).is_empty());
    }

    /// Regression (#228): on the reporter's machine the renderer is turned off, but never while
    /// another thread runs: `set_var` would not be sound then. This test runs beside other threads
    /// (the test harness, and the one it holds), so nothing is set and the environment stays as it
    /// was; the binary, alone at that point, sets it (`tests/cli.rs`).
    #[test]
    fn regression_the_reporter_s_machine_turns_the_renderer_off_but_not_beside_other_threads() {
        let root = tempfile::tempdir().unwrap();
        drm(root.path(), &["nouveau", "i915"]);
        let before = std::env::var_os(WEBKIT_DISABLE_DMABUF_RENDERER);
        let (release, held) = std::sync::mpsc::channel::<()>();
        let other = std::thread::spawn(move || {
            let _ = held.recv();
        });
        let startup = prepare_in(root.path(), env(GNOME_WAYLAND));
        release.send(()).unwrap();
        other.join().unwrap();
        assert_eq!(startup.drivers, ["nouveau", "i915"]);
        assert!(startup.wayland);
        assert_eq!(startup.renderer, DmabufRenderer::Disable(NvidiaDriver::Nouveau));
        assert!(matches!(startup.outcome, Outcome::NotAlone(Some(threads)) if threads >= 2), "{:?}", startup.outcome);
        assert_eq!(std::env::var_os(WEBKIT_DISABLE_DMABUF_RENDERER), before, "nothing set beside other threads");
        let line = logged(&startup);
        assert!(line.contains("WARN") && line.contains("other threads run already") && line.contains("is not set"), "{line}");
        assert!(line.contains("driver=nouveau") && line.contains(r#"gpus=["nouveau", "i915"]"#), "{line}");
    }

    /// The line for the renderer turned off names the variable and the drivers found, and nothing
    /// about the machine.
    #[test]
    fn the_line_for_the_renderer_turned_off_names_the_variable_and_the_drivers() {
        let startup = Startup {
            drivers: vec!["nouveau".into(), "i915".into()],
            wayland: true,
            renderer: DmabufRenderer::Disable(NvidiaDriver::Nouveau),
            user_value: None,
            outcome: Outcome::Set,
        };
        let line = logged(&startup);
        assert!(line.contains("INFO") && line.contains("WEBKIT_DISABLE_DMABUF_RENDERER=1") && line.contains("driver=nouveau"), "{line}");
        assert!(line.contains(r#"gpus=["nouveau", "i915"]"#), "{line}");
        assert_eq!(line.lines().count(), 1, "{line}");
    }

    /// X11, `GDK_BACKEND=x11` and a machine without an NVIDIA GPU change nothing.
    #[test]
    fn x11_and_other_gpus_change_nothing() {
        let nouveau = tempfile::tempdir().unwrap();
        drm(nouveau.path(), &["nouveau", "i915"]);
        let intel = tempfile::tempdir().unwrap();
        drm(intel.path(), &["i915"]);
        let x11: &[(&str, &str)] = &[("DISPLAY", ":0"), ("XDG_SESSION_TYPE", "x11")];
        let gdk_x11 = [GNOME_WAYLAND, &[("GDK_BACKEND", "x11")]].concat();
        for (root, vars, wayland) in [(nouveau.path(), x11, false), (nouveau.path(), gdk_x11.as_slice(), false), (intel.path(), GNOME_WAYLAND, true)] {
            let startup = prepare_in(root, env(vars));
            assert_eq!((startup.wayland, startup.renderer, startup.outcome), (wayland, DmabufRenderer::Keep, Outcome::Unchanged), "{vars:?}");
            let line = logged(&startup);
            assert!(line.contains("DEBUG") && line.contains("left as it is") && !line.contains("WEBKIT_DISABLE_DMABUF_RENDERER"), "{line}");
        }
    }

    /// The user's own value stays, and the line says what it was.
    #[test]
    fn a_value_the_user_set_is_kept_and_reported() {
        let root = tempfile::tempdir().unwrap();
        drm(root.path(), &["nouveau", "i915"]);
        let vars = [GNOME_WAYLAND, &[(WEBKIT_DISABLE_DMABUF_RENDERER, "0")]].concat();
        let startup = prepare_in(root.path(), env(&vars));
        assert_eq!(startup.renderer, DmabufRenderer::UserChoice(NvidiaDriver::Nouveau));
        assert_eq!(startup.outcome, Outcome::Unchanged);
        assert_eq!(startup.user_value.as_deref(), Some(std::ffi::OsStr::new("0")));
        let line = logged(&startup);
        assert!(line.contains("INFO") && line.contains("stays as the user set it") && line.contains(r#"value="0""#), "{line}");
        assert!(!line.contains("WEBKIT_DISABLE_DMABUF_RENDERER=1"), "{line}");
    }

    /// Only a debug build reads the override, and an empty one means `/`.
    #[test]
    fn the_gpu_root_is_slash_unless_a_debug_build_names_another() {
        assert_eq!(gpu_root(None, true), Path::new("/"));
        assert_eq!(gpu_root(Some("".into()), true), Path::new("/"));
        assert_eq!(gpu_root(Some("/tmp/gpus".into()), true), Path::new("/tmp/gpus"));
        assert_eq!(gpu_root(Some("/tmp/gpus".into()), false), Path::new("/"));
        assert_eq!(gpu_root(None, false), Path::new("/"));
    }
}
