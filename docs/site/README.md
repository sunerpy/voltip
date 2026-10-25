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
| `public/` | site root (`/voltip-logo.svg`, `/screens/*.webp`, `/community/*`) |
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
| `<VideoFigure src poster width height title caption? />` | a video with its poster; nothing loads until it plays |
| `<QrCode src alt caption? size? />` | a QR code on a white plate in both themes |
| `<Badge>` | VitePress's own badge |
| `<TryVoltip />` | the online try page (`guide/try.md`) only, a `layout: page` with `pageClass: vt-try-page` and `sidebar: false`, so no sidebar or outline competes with it; its API is firlab's `voltip/functions/`, and its own words (buttons, the line on what is kept, the link to the app) are firlab's `try/strings.ts` |
| `HomeFlow`, `HomePairs`, `HomePresets`, `HomeModes`, `HomeMore`, `HomePhone`, `HomePlatforms`, `HomeFaq`, `HomeCloser` | the home pages only, in this order; each renders its part of the `home:` frontmatter. `HomePlatforms` wraps the install commands' code group, which stays Markdown |

## The home pages

The words of both home pages live in their frontmatter: VitePress's `hero:` (text, tagline,
buttons; no `name`) and a `home:` block that the components render. The pages set
`markdownStyles: false`, because the bands below the hero are not prose. The build checks
`home:` against `voltip/src/.vitepress/theme/data/home-schema.ts` in firlab and fails on a
missing field or an unknown one, and on a dead link anywhere in `hero:` or `home:`.

| Key | Holds |
| --- | --- |
| `checks` | the short facts under the buttons, at least two |
| `scene` | the hero's picture, one real dictation: `steps` (what the reader does), `live` (the overlay while speaking), `label` and `result` (the text inserted at the cursor), `inserted` (the overlay after inserting), `caption` |
| `try` | the band under the hero that leads to the try page: `title`, `body`, a `heard` → `polished` sample, `action` (`text`, `link`) |
| `flow` | how one dictation is processed, optional: `title`, `intro`, `steps[]` (`icon`, `name`, `text`, `tag`; at least three), `note` |
| `pairs` | `title`, `intro`, `labels`, `items[]`: `kind` (`dictation`, or `edit` with a `selection`), `source` (`actual` or `example`), `said`, `result`; `note` |
| `presets` | `title`, `intro`, `points`, `saidLabel`, `said`, `outputs[]` (`name`, `desc`, `text`; at least two), `note`, `scenes` (`title`, `note`, `labels` (`preset`, `instruction`), `items[]`: `name`, `preset`, `apps[]` (`name`, with an app `mark` or an `icon`), `more`, the apps without a mark in words, `tools` (`label`, `apps[]` as above and `more`, the tools without a mark in words: tools that run inside the scene's apps, such as coding agents in a terminal, so the scene applies through the app around them), `instruction`, the scene's extra instructions word for word as the app has them, and `body`; at least one of `apps`, `instruction` and `body`) |
| `modes` | `title`, `intro`, `scope`, `items[]`: `icon`, `label`, `badge`, `title`, `flow` (`from`, `to`, `icon`, `local`), `assure`, `specs[]` (`term`, `text`), `chipsLabel`, `chips[]` (`name`, with a `mark` or an `icon`); `polish` (`title`, `body`, `marks[]`); `note` |
| `more` | more features, optional: `title`, `intro`, `items[]` (`title`, `body`, `wide` for a cell two columns wide, at most one of `keys[]`, `chips[]`, `chain[]`, `code` and `swap` (`from`, `to`), and `guide`, a link to the guide page); `note` |
| `phone` | `title`, `intro`, `items[]` (`icon`, `title`, `body`), `figure` (`back`, `front`, `caption`), `links` |
| `platforms` | `title`, `intro`, `cards[]`: `name`, `status`, `specs[]`; `install`, the line under the commands; `note` |
| `faq` | `title`, `intro`, `items[]`: `q`, `a` |
| `closer` | `title`, `body`, `actions[]`: `text`, `link`, `theme` (`brand` or `alt`), `icon`; `keys[]`, the shortcut as key caps |

An image is `src`, `dark` (the same screen in the dark theme, optional), `width`, `height` and
`alt`. `icon` and `mark` take the names listed in `home-schema.ts` (`ICONS`, `MARKS` for the
providers, `APP_MARKS` for the scenes' apps). A mark is the vendor's or the app's logo in its own
colours, from Lobe Icons or Simple Icons, or the owner's own icon file shown as it is (`vscode`,
which links to the VS Code website as its terms ask, and `libreoffice-writer`); an app no open
icon set carries a mark for, or whose owner's terms forbid it (Apple's apps, Microsoft's other
products, for example), is named in `more`, never drawn. Status is one of `available`, `building` or
`planned`.

A few fields take marks; every other character is shown as written:

- `\n` in a quoted string breaks the line: in the band titles, `said`, `selection`, `result`
  and `outputs[].text`.
- `~~words~~` strikes out what polish removes: in `said`, `selection` and `try.heard`.
- `[text](/path)` is a link to a site path, an `#anchor` or an `https://` address: in `said`,
  `selection`, `modes.note`, `phone.links`, `platforms.install`, `faq.intro`,
  `faq.items[].a`, `flow.note`, `more.items[].body`, `more.items[].guide` and `more.note`.

A result marked `actual` is what Voltip inserted in a screen recording, word for word. The
`scene` comes from such a recording too: its overlays are frames of it and its `result` is
the text it inserted. Anything written for the page is an `example` and says so. The home
pages carry no version number: the install line links to the releases page.

## Screenshots

Screenshots are WebP files in `public/screens/`, named `<page>-<lang>-<theme>.webp`. They come
from the desktop front end running on the mock backend, which shows no real host, except the
hero's overlay frames, which come from a screen recording (step 5):

```bash
pnpm -C apps/desktop dev --host 127.0.0.1 --port 1430
```

1. Open it at 1440 × 900 and navigate inside the app: a full reload resets the mock.
   Switch the theme with the sidebar's theme button — 澄空 (Clear Sky) for the `light`
   captures, 夜空 (Night Sky) for the `dark` ones — and the language in 设置 › 通用. The
   overlay sheet has no sidebar: go back home to switch, then return to `/overlay`.
2. Page captures are viewport screenshots at device pixel ratio 1, saved as WebP at
   quality 85.
3. The overlay pills, `overlay-<state>-<lang>-<theme>.webp`, come from `/overlay` at device
   pixel ratio 2; firlab's `og.svg` uses `overlay-inserted-en-light.webp`. For each language
   and theme, save a viewport screenshot as `<dir>/overlay-<lang>-<theme>.png`, and the pill
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

   Then cut them out onto a canvas at least as wide as the widest pill, and check `og.svg`
   after a new cut, since it gives the image a fixed size:

   ```bash
   python3 docs/site/tools/crop-overlay.py --width 440 --out docs/site/public/screens <dir>/overlay-*-*.png
   ```

4. The home pages' phone panels, `phone-pair-<lang>-<theme>.webp` and
   `phone-mic-<lang>-<theme>.webp`, are the pairing card and the phone microphone card of
   the 手机 page, where the mock has a paired phone online. Capture the page at device pixel
   ratio 2 as `<dir>/phone-<lang>-<theme>.png`, read each card's `getBoundingClientRect()`,
   double it, and cut the card with its 24 px corners:

   ```bash
   python3 docs/site/tools/crop-card.py --rect <x,y,width,height> --radius 48 \
       <dir>/phone-zh-light.png docs/site/public/screens/phone-pair-zh-light.webp
   ```

   Update `width` and `height` under `phone.figure` in that language's home page when a
   card changes size.
5. The hero's overlay frames, `hero-live-<lang>.webp` and `hero-inserted-<lang>.webp`, are
   frames of a screen recording of a real dictation at 2× scale, the kind the tutorial
   videos are made from: one while the overlay shows the words being said, one after it
   reports the text inserted. Save a frame with
   `ffmpeg -ss <seconds> -i <recording> -frames:v 1 <dir>/frame.png` and cut the overlay as
   a capsule:

   ```bash
   python3 docs/site/tools/crop-card.py --rect <x,y,width,height> --pill \
       <dir>/frame.png docs/site/public/screens/hero-live-en.webp
   ```

   Then update `scene.live` and `scene.inserted` (size and `alt`) in that language's home
   page, and make `scene.result` and the first `actual` pair the text that recording
   inserted.
6. Look at every image before committing it. The host guard does not read images.

Capture again when the interface's text or layout changes. The READMEs show
`home-en-light.webp` and `home-zh-light.webp` too: a new capture updates them as well, and a
rename has to change both READMEs.

## Videos and QR codes

The tutorial videos are too large for this repository: the files live in firlab under
`voltip/src/public/media/` (`voltip-tutorial-<lang>.mp4` and its `.webp` poster), and the
pages only give the path. Keep each file under Cloudflare Pages' 25 MiB limit and encode it
with `-movflags +faststart`, so it starts playing before it has finished downloading. The
captions are part of the picture.

The READMEs embed 720p copies uploaded as GitHub attachments (GitHub plays only those, up to
10 MB). Upload a new copy by dropping it into a comment box of this repository and posting the
comment, then replace the `https://github.com/user-attachments/assets/…` line in both READMEs.
Check that the link opens without logging in: `curl -sI <link>` answers 302, while 404 means
the file is visible to signed-in users only.

`public/community/` holds the three codes of the community page, which the READMEs use too:
`telegram-group.png`, `wechat-group.png` and `wechat-official-account.jpg`. Check that a code
still scans after every edit.

- The Telegram code holds the same invite link as the text link beside it: if the link
  changes, replace the code and the link in both community pages and both READMEs together.
  Telegram's own export leaves less than one module of margin, so pad it with white to a
  quiet zone of four modules.
- The WeChat group's code expires after seven days, so replace `wechat-group.png` every week,
  under the same name: crop WeChat's share card to the code with a quiet zone of four
  modules, and drop the members' avatars.

## Reviews

- **2026-09-29, copy (Sepia refactor: professional pass and style pass).** Author and executor:
  Claude, release not in Sepia's tables, so its Claude prose layers applied as priors. Venue
  corpus: the app's own interface copy (`packages/shared/src/i18n/`) and this file's rules. The
  vocabulary scan over every user page found two technical uses (a silent install, UAC
  elevation), no cluster. The checklist failed three things, all fixed: a "Not X / Not Y / Not Z"
  list on *What is Voltip* (now two plain paragraphs), an idiom on the home page ("at a glance"),
  and three pages without an opening sentence that says what they help with (quick start,
  updates, FAQ). Two measured timings were missing their hardware (SenseVoice's CPU timing, the
  first run on a graphics card in the FAQ); both now name it. The same review also stopped the
  pages from presenting two unfinished things as done: permissions kept across macOS updates
  between fixed-certificate releases (not yet checked on a real Mac, see
  `docs/acceptance/macos/manual-checklist.md` item 15) and iOS, which is now listed as planned.

