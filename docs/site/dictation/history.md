---
description: The dictation history on your computer, what each entry shows, how much is kept, and the statistics on the home page.
---

# History

Every dictation, voice edit and text sent from a paired phone is listed on the **History** page. The history is stored only on your computer.

The latest results are also listed on the home page: six, or more when the window is taller, up to 30.

## What an entry shows

- The text that was inserted and the text that was recognised, with a view of what changed between them.
- Which dictionary corrections and replacement rules applied, and the app and [scene](/recognition/scenes) the dictation was for.
- The [AI preset](/recognition/polish#presets) that AI polish used, when it ran.
- The models used and how long each step took.
- Where the text went: pasted at the cursor, or left on the clipboard, with the reason.

From an entry you can:

- copy it, or paste it into the window you used before (see [Where the text goes](/dictation/output#copy-or-paste-a-result-again));
- star it, so that it is easy to find;
- add a misheard word to the [dictionary](/recognition/dictionary) with **Add to dictionary**;
- delete it.

The search box finds text, apps, scenes and models. Filters narrow the list to today, this week or this month, to starred entries, or entries whose text was not inserted. The list shows 100 entries at a time; scroll to the end, or select **Load more**, to see the next ones.

## Long recordings

An entry recorded for longer than 2 minutes, or with more than 2,000 characters, has two more tools in its detail:

- **Process with an AI preset**: choose a preset and select **Process**. The text is processed in parts of up to 1,500 characters, cut at sentence ends; the progress is shown and you can cancel. The result is saved as a processed text beside the original, which stays as it is, and appears under **Processed**. With Key points, the points of all parts are summarised once more when they fit one request; with Chinese ⇄ English, you get a translation. The built-in AI service runs on a free quota, so long texts take a while.
- **Export subtitles (SRT)** and **Export text (TXT)**: choose where to save the file. Subtitles follow the segments of the recording, with at most 20 Chinese or 42 Latin characters per line. The text export uses the processed text when there is one.

## How much is kept

Under **Settings → Privacy and history**:

- **Keep the latest** sets how many entries are kept: 500, 2,000, 5,000, 10,000 or 20,000. A new installation keeps 20,000, which is also the maximum. Older entries are removed.
- Updating does not change this setting: if it was 500, it stays 500 until you choose a larger number.
- History can be turned off. Nothing new is recorded; what is already there stays until you clear it.
- **Clear history** deletes all entries, after a confirmation.

## Statistics

The home page adds up the dictations in the history for today, this week, this month and in total:

- **Characters transcribed**: the characters the speech model recognised.
- **Characters corrected**: how many characters differ between the recognised text and the text inserted, spaces not counted.
- **Speaking time** and **Time saved**.

Time saved is speaking time × 1.9. The factor comes from Ruan et al., 2016 (arXiv:1608.07323): in their experiments, speaking was about 2.9 times as fast as typing on a phone, in English (153 against 52 words per minute) and in Chinese (123 against 43 characters per minute). Typing the same text takes about 2.9 times as long as saying it, which saves 1.9 times the speaking time. **Basis**, next to **Time saved** on the home page, shows the same explanation.

Only dictations kept in the history count. Voice edits, and text or clipboard content sent from the phone, do not; a recording made on the phone and recognised on this computer does.
