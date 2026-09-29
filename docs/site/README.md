# docs/site — the pages of voltip.firlab.app

This directory holds the words of the Voltip website: English at `docs/site/`, Chinese at
`docs/site/zh/`, one file per page and the same path in both languages. Everything else
(the VitePress configuration, the theme, the components and the deployment) lives in
[`sunerpy/firlab`](https://github.com/sunerpy/firlab) under `voltip/`.

| Path | Published at |
| --- | --- |
| `index.md`, `zh/index.md` | `/`, `/zh/` (home pages) |
| `guide/`, `dictation/`, `recognition/`, `phone/`, `reference/` | the user guide |
| `privacy.md`, `roadmap.md`, `developers.md` | reference pages |
| `docs/{architecture,dictation,frontend,pairing,protocol,threat-model,state-machines,feedback}.md` | `/zh/dev/*`, as they are; `/dev/*` is a generated English pointer |
| `public/` | site root (`/voltip-logo.svg`, `/screens/*.webp`) |
| `tools/` | helpers for authors; not published |

## How a change reaches the site

1. A pull request that touches these files runs `.github/workflows/docs-site.yml`: it
   checks out the public firlab repository, syncs this directory into it and builds the
   site. A dead link, an unknown component or a malformed home page fails the check.
2. After the merge, `.github/workflows/publish-site.yml` runs firlab's
   `voltip/scripts/sync-voltip-docs.sh` and commits the result to firlab's `main` as
   `docs(voltip): sync from voltip@<sha>`.
3. firlab's `deploy-voltip.yml` builds the site and deploys it to Cloudflare Pages.

The footer of every page names the Voltip commit its content came from.

## Preview

```bash
git clone https://github.com/sunerpy/firlab ../firlab    # once
../firlab/voltip/scripts/sync-voltip-docs.sh "$PWD"
cd ../firlab/voltip && pnpm install --frozen-lockfile && pnpm dev
```

Run the sync again after each edit. It stops with a message when a page has no
counterpart in the other language, uses a component the site does not register, or uses
a word from the lists below.

## Writing

- Say what the reader does, sees and gets. User pages never name the internals: no
  crate or file names, no `§`, IPC, `UiState` or `voltip-core`. `developers.md` is the
  exception.
- Use standard, readable written language, the register of Apple's and Microsoft's
  Chinese documentation: 「未设置」「尚未下载」「无法连接」「仅保存在本机」. No colloquial
  words (还没、没能、免得、搭的、咋、啥; just, gonna, stuff), no stacked explanations, no
  promises in the voice of a person. The sync script rejects the colloquial words and
  the internal terms of `packages/shared/src/i18n/copy.test.ts`; keep its lists in step
  with that test.
- One word per concept, the word the app uses: 首页, 历史记录, 词典, 规则, 语音模型,
  AI 模型, 手机, 悬浮窗, 快捷键, 插入. 「本地」 is processing on this computer (本地模型,
  本地识别), 「本机」 is this device.
- Chinese pages keep English only for product names, model names and keys, with a space
  between Chinese and Latin text. Headings end without a full stop.
- The first sentence of a page says what the page helps with.
- Every number has a source: model sizes come from the model catalogue, measured
  timings name the hardware.
- Both languages have the same sections in the same order.
- A feature that is not released goes in its own section with
  `<StatusTag status="building" />` and is never described as available.
- No real host name, token or key, in the text or in a screenshot (`AGENTS.md`,
  "Secrets and hosts").

## Components

Pages may use these components and no others:

| Component | Use |
| --- | --- |
| `<StatusTag status="available \| building \| planned" />` | release state, shown as text |
| `<ScreenFigure src dark? width height alt caption? />` | a screenshot; `dark` is the same screen in the dark theme |
| `<Badge>` | VitePress's own badge |
| `HomeIndex`, `HomeSteps`, `SplitBlock`, `HomeModels`, `HomePlatforms`, `HomePrivacy`, `HomeRoadmap` | the home pages only; they render the `home:` frontmatter |

## The home pages

The words of both home pages live in their frontmatter: VitePress's `hero:` (name, text,
tagline, buttons) and a `home:` block that the components render. The build checks
`home:` against `voltip/src/.vitepress/theme/data/home-schema.ts` in firlab and fails
on a missing field or an unknown one.

| Key | Holds |
| --- | --- |
| `facts` | the lines under the tagline: `term`, `text` |
| `visual` | `home` (the app's home page, light and dark) and `pill` (the overlay's three states) |
| `index` | `title`, `intro`, `groups[].items[]`: `title`, `body`, `status`, `link` |
| `steps` | `title`, `items[]`: `title`, `body`, `keys` |
| `models`, `polish`, `phone` | the evidence beside the three split blocks |
| `platforms` | `columns`, `rows[]`: `name`, `status`, `cells` (one fewer than `columns`), `note` |
| `privacy` | `modes[]`: `name`, `sends`, `detail` |
| `roadmap` | `items[]`: `title`, `body`, `status`; `notPlanned` |

Status is one of `available`, `building` or `planned`. The home pages carry no version
number: the buttons link to the releases page.

## Screenshots

Screenshots are WebP files in `public/screens/`, named `<page>-<lang>-<theme>.webp`, and
come from the desktop front end running on the mock backend, which shows no real host:

```bash
pnpm -C apps/desktop dev --host 127.0.0.1 --port 1430
```

1. Open it at 1440 × 900 and navigate inside the app: a full reload resets the mock.
   Switch the theme in the sidebar and the language in 设置 › 通用.
2. Page captures are viewport screenshots at device pixel ratio 1, saved as WebP at
   quality 85.
3. The overlay comes from `/overlay` at device pixel ratio 2. For each language and
   theme, save a viewport screenshot as `<dir>/overlay-<lang>-<theme>.png`, and the pill
   rectangles as `<dir>/overlay-<lang>-<theme>.json` from the page:

   ```js
   JSON.stringify({
     dpr: devicePixelRatio,
     rects: Object.fromEntries(['listening', 'processing', 'inserted'].map((state) => {
       const r = document.querySelector(`[data-state="${state}"].rounded-pill`).getBoundingClientRect();
       return [state, { x: r.x, y: r.y, w: r.width, h: r.height }];
     })),
   })
   ```

   Then cut them out onto a canvas at least as wide as the widest pill:

   ```bash
   python3 docs/site/tools/crop-overlay.py --width 392 --out docs/site/public/screens <dir>/overlay-*-*.png
   ```

   Update `width` and `height` under `visual.pill` in both home pages if the canvas
   changes.
4. Look at every image before committing it. The host guard does not read images.

Capture again when the interface's text or layout changes.
