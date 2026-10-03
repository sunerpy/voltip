// Class lists the controls add under the touch presentation (`PresentationProvider value="touch"`)
// and never otherwise, so the desktop's markup stays as it was. Each is written out in full: the
// apps' Tailwind finds class names by scanning these files. Hover needs nothing here: Tailwind 4
// applies `hover:` only under `@media (hover: hover)`, so on a phone no hover style sticks after a
// tap, and the pressed (`active:`) states take its place.

/** No grey flash where a finger lands (Android's WebView paints one over anything that takes a
 *  click). The property is inherited: on a wrapper it covers everything inside it. */
export const NO_TAP_HIGHLIGHT = "[-webkit-tap-highlight-color:transparent]";

/** A control under the touch presentation: no tap flash, and no double-tap zoom on it. */
export const TOUCH_CONTROL = "touch-manipulation [-webkit-tap-highlight-color:transparent]";

/** A touch target of at least 44 × 44 px for a control drawn smaller (Apple's 44 pt, WCAG 2.5.5):
 *  an invisible box centred on the control takes the taps, so the control keeps its size and its
 *  place (GitHub Primer's touch target works the same way). It needs a positioned control. */
export const TOUCH_TARGET =
  "relative after:absolute after:top-1/2 after:left-1/2 after:h-[max(100%,44px)] after:w-[max(100%,44px)] after:-translate-1/2";

/** The same box grown in height only, for segments drawn side by side: grown in width too, one
 *  segment's box would cover its neighbour. */
export const TOUCH_TARGET_Y =
  "relative after:absolute after:inset-x-0 after:top-1/2 after:h-[max(100%,44px)] after:-translate-y-1/2";

/** The touch target of a text field: drawn before the field's content, under the `<input>` (which
 *  is positioned for that), so a tap on the text still places the caret; a tap on the rest of the
 *  box focuses the input (`Input` does that). */
export const TOUCH_FIELD_TARGET =
  "relative before:absolute before:inset-x-0 before:top-1/2 before:h-[max(100%,44px)] before:-translate-y-1/2";
