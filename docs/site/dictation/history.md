---
description: The dictation history on your computer, what each entry shows, how much is kept, the statistics on the home page, and backing up and moving to another computer.
---

# History

Every dictation, voice edit and text sent from a paired phone, and every copy a phone uploads of what it transcribed on its own, is listed on the **History** page. The history is stored on your computer; paired phones can read it, see [The computer's history and settings](/phone/#the-computer-s-history-and-settings).

The latest results are also listed on the home page: four, or more when the window is taller, up to 30.

## What an entry shows

- The text that was inserted and the text that was recognised, with a view of what changed between them.
- Which dictionary corrections and replacement rules applied, and the app and [scene](/recognition/scenes) the dictation was for.
- The [AI preset](/recognition/polish#presets) that AI polish used, when it ran.
- The models used and how long each step took.
- Where the text went: pasted at the cursor, or left on the clipboard, with the reason.

From an entry you can:

- copy it, or paste it into the window you used before (see [Where the text goes](/dictation/output#copy-or-paste-a-result-again));
- star it, so that it is easy to find;
- select a misheard word in the original text and add it to the [dictionary](/recognition/dictionary) or create a rule, see [Correct from the history](/recognition/dictionary#correct-from-the-history);
- delete it.

The search box finds text, apps, scenes and models, and works together with the filters, which narrow the list to today, this week or this month, to starred entries, or to entries whose text was not inserted. After a search or a filter the list starts again at its first page, and the foot of the list shows how many entries were found.

On the computer the list is shown in pages. Choose 20, 50, 100 or 200 entries per page at the foot of the list (50 at first; the page remembers your choice) and move between pages with the buttons beside it. A new dictation does not change the page you are on. While you scroll through a page, the details of the selected entry stay in view beside it; when they are longer than the window, they scroll on their own. On a phone the list shows 100 entries at a time; scroll to the end, or tap **Load more**, to see the next ones.

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

Only dictations kept in the history count. Voice edits, and text or clipboard content sent from the phone, do not; a recording made on the phone and recognised on this computer does, and so does a copy a phone uploads of what it transcribed on its own.

## Backup and moving to another computer

Under **Settings → Privacy and history**, **Backup and migration** exports what is on this computer into one backup file, to keep as a backup or to move to another computer.

A backup contains:

- the whole history. The statistics on the home page, such as the time saved and the characters, are worked out from it, so they come back with it;
- the dictionary, the rules, the scenes (with the settings of the built-in scenes) and your own AI presets.

A backup does not contain settings, API keys, paired devices or local models. On the new computer, choose the speech model and the AI model again, enter your keys and pair your phone.

### Export a backup

1. Open **Settings → Privacy and history** and select **Export…** under **Backup and migration**.
2. Choose where to save the file. Its name is **Voltip backup** and the date, with the extension `.voltip-backup`.

The backup file is not encrypted and its dictations can be read as they are: keep it as safe as the history itself.

### Import a backup

1. On the other computer, install Voltip, open **Settings → Privacy and history**, select **Import…** under **Backup and migration** and choose the backup file.
2. Voltip shows the device the backup comes from, when it was made and how many items each part holds; nothing has changed yet. Choose the parts to import and how:
   - **Merge** keeps what is on this computer and adds what is new in the backup. Entries already here, dictionary entries with the same spelling, and rules, scenes and presets with a name already in use are skipped, as is anything past a limit (500 dictionary entries, 200 rules, 50 scenes, 30 AI presets). Built-in scenes keep this computer's settings.
   - **Replace** deletes what this computer has in the chosen parts and puts the backup's content in its place.
3. Select **Import** (**Replace** when replacing). Voltip then lists, part by part, how many items it imported, and how many it skipped and why.

The history keeps as many entries as **Keep the latest** allows; the older ones beyond that are not kept. To import the whole history, set **Keep the latest** high enough first.

Between operating systems the same app has a different identifier, so choose the apps of your own scenes again; the default apps of the built-in scenes switch to the new system's.

When the chosen file is not a Voltip backup, is damaged, was made by a newer version of Voltip or is larger than 512 MB, Voltip says why and imports nothing.
