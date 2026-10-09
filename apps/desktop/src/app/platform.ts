import type { Platform } from "@voltip/shared";
import type { TitleBarPlatform } from "@voltip/ui";

/**
 * Host platform for the self-drawn title bar.
 *
 * Deliberately **not** `@tauri-apps/plugin-os`: that costs an npm package, a Cargo crate, a
 * `.plugin(...)` call and a capability entry to answer a question the webview's own user agent
 * answers synchronously, and the plugin is not injected under vitest at all.
 *
 * Order matters: mobile agents are screened first because Android's UA also contains `Linux`
 * and iOS's contains `Mac OS X`; `Windows` / `Linux` come before anything Safari-shaped because
 * WebKitGTK (the Linux webview) reports `AppleWebKit … Safari` too. Anything not positively
 * identified is `unknown`, which the title bar renders with trailing controls and no inset.
 */
export function platformFromUserAgent(userAgent: string): TitleBarPlatform {
  if (/iPhone|iPad|iPod|Android/i.test(userAgent)) return "unknown";
  if (/Mac OS X|Macintosh/i.test(userAgent)) return "macos";
  if (/Windows/i.test(userAgent)) return "windows";
  if (/Linux|X11|CrOS|BSD|SunOS/i.test(userAgent)) return "linux";
  return "unknown";
}

/** Maps the core's `identity.platform` (Rust `std::env::consts::OS`) onto the title-bar union. */
export function platformFromIdentity(platform: Platform | undefined): TitleBarPlatform {
  return platform === "macos" || platform === "windows" || platform === "linux"
    ? platform
    : "unknown";
}

/** User agent first; the core identity only breaks a tie the UA could not resolve. */
export function resolvePlatform(
  hint: Platform | undefined,
  userAgent: string | undefined = currentUserAgent(),
): TitleBarPlatform {
  const fromUa = typeof userAgent === "string" ? platformFromUserAgent(userAgent) : "unknown";
  return fromUa === "unknown" ? platformFromIdentity(hint) : fromUa;
}

function currentUserAgent(): string | undefined {
  return typeof navigator === "undefined" ? undefined : navigator.userAgent;
}
