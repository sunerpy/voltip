# The phone app's icon

The Voltip mark (`docs/site/public/voltip-logo.svg`) as the Android launcher icon. Regenerate
every density after changing a source:

```bash
cd apps/mobile/src-tauri && cargo tauri icon icon-source/icon.json
```

The command writes `gen/android/app/src/main/res/mipmap-*` (the adaptive icon with its
foreground, background and monochrome layers, and the plain icons) and `icons/icon.png`. It also
writes desktop and iOS icons into `icons/` that the phone app does not use: delete them.

- `voltip.svg`: the whole mark.
- `voltip-background.svg`: the mark's navy, full bleed; the launcher masks the shape.
- `voltip-foreground.svg`, `voltip-monochrome.svg`: the V alone, scaled to two thirds so that it
  stays inside the 66 dp safe circle of the 108 dp canvas. The generator draws an SVG canvas
  whole and ignores `android_fg_scale` for it, so the scale lives in the sources.
