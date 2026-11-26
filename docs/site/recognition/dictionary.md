---
description: The personal dictionary corrects words the recogniser gets wrong; replacement rules rewrite the final text; both run on your computer.
---

# Dictionary and rules

Two lists on the sidebar change the recognised text before it is inserted. Both run on your computer, and if either fails for any reason the dictation continues with the text as it was.

## Dictionary

The **Dictionary** page lists words the recogniser tends to get wrong. Each entry has:

- the **right spelling**, for example `kubectl` or a colleague's name;
- up to 10 spellings it is **heard as**, for example `cube control`.

Right after recognition, every heard-as spelling in the text is replaced with the right one, before AI polish and the rules. The right spellings of enabled entries also go as a glossary to AI polish, and to recognition services that take one, which keep them as written:

| Recognition service | Glossary |
| --- | --- |
| The built-in service | Sent with the recording |
| OpenAI, Groq, SiliconFlow and custom endpoints | Sent with the recording |
| Alibaba Cloud Model Studio's qwen-audio models | Sent as hot words |
| Model Studio's fun-asr-realtime and fun-asr-flash | Sent as context |
| Model Studio's other speech models, such as qwen3-asr-flash and paraformer-realtime-v2 | Not sent |
| Volcengine Doubao Speech, Tencent Cloud Speech Recognition, Zhipu GLM's glm-asr-2512, StepFun stepaudio-2.5-asr | Sent as hot words |
| StepFun stepaudio-2.5-asr-stream | Sent as a prompt |
| iFlytek Dictation, iFlytek Realtime Transcription, Baidu AI Cloud Speech, Xiaomi MiMo's mimo-v2.5-asr | Not sent; iFlytek's hot words can be uploaded for the app in the iFlytek console |
| Local models | Not sent |

With a recognition service that takes no glossary, the dictionary works through the corrections alone. An enabled built-in scene also adds its terms to the glossary in its apps, so a dictation there can send a glossary even when no dictionary entry is enabled. While a fallback model recognises in place of the selected one (its quota ran out, or it failed with **Also switch when a service fails** on), what happens to the glossary depends on the fallback model. The top of the Dictionary page, and the Speech models page under the recognition services, say whether the service in use receives the glossary; in the history, an entry's **Glossary** line says whether it went to recognition that time.

Matching works like this:

- Letters are matched regardless of case.
- In languages written with spaces, only whole words match: `cat` is not replaced inside `concatenate`.
- Chinese, Japanese and Korean text is matched anywhere in the text.
- The longest match wins, and a replaced word is not matched again.

The **Try it** box on the page shows what the dictionary changes in a sample text; **Use the last dictation** fills it with your latest recording. You can also correct a misheard word right in the history, see [Correct from the history](#correct-from-the-history).

<ScreenFigure src="/screens/dictionary-en-light.webp" width="1440" height="900"
  alt="The Dictionary page with three entries, and the Try it box showing a sample sentence before and after the corrections."
  caption="Three entries, and their corrections in the Try it box." />

The dictionary holds up to 500 entries. Their order is their priority when the glossary has to be shortened.

## Replacement rules

The **Rules** page rewrites the final text, after AI polish, in the order of the list. Each rule has:

- a name;
- a **literal** pattern or a **regular expression**;
- the replacement, which may be empty to delete the match;
- whether letter case must match.

Regular expressions use Rust syntax: groups can be referred to as `$1`, but look-around and back-references are not available. A rule that fails to compile is rejected when you save it.

Rules can be imported from and exported to a TOML file, for example:

```toml
version = 1

[[rule]]
name = "git push"
kind = "literal"
pattern = "give it push"
replacement = "git push"

[[rule]]
name = "PR number"
kind = "regex"
pattern = '\bpr (\d+)'
replacement = "PR #$1"
case_sensitive = false
```

Importing either replaces the list or merges into it, matching rules by name. The rules list holds up to 200 rules. **Test run** passes a sample text through the dictionary and every enabled rule, without inserting anything.

The rules list points out three problems under the rule concerned: another rule finds the same text, so only the one higher in the list gets hits; two rules make opposite replacements, so text can be changed back; a rule turns the right spelling of a dictionary entry back into one of its heard-as forms, undoing the correction.

## Correct from the history

Open an entry in **History** and switch to the **Original** view: it shows the text as the recogniser wrote it, which is where misheard words are.

- Words AI polish already corrected have a dotted blue underline; hovering shows the polished spelling, for example "Polished as “Voltip”". Selecting one opens a prefilled dictionary card. Changes the dictionary and the rules made are not included; if an entry or rule used on the record was deleted or changed later, Voltip can no longer tell which words AI polish changed, and the record shows no such underline.
- Words that are already the heard-as form of an entry have a solid green underline.
- Select any other misheard word and a toolbar appears above it: **Add to dictionary** (D) or **New rule** (R). Double-click selects a word. When the selection is longer than 64 characters or spans lines, **Add to dictionary** is unavailable and says why.

<ScreenFigure src="/screens/history-fix-en-light.webp" width="1440" height="900"
  alt="The Original view in History: 维他命 and 酷伯内特斯 have a dotted blue underline; the card under 维他命 has it as Heard as and Voltip prefilled as Right spelling."
  caption="Selecting a word AI polish corrected: the right spelling comes from the polished text." />

The card opens under the selection, without leaving the page:

- **Heard as** is the selected text; **Right spelling** is prefilled from the polished text where possible, marked **From the polished text**, so Enter saves it.
- The card tells you what the change would affect, for example "Besides this one, 3 of the last 499 entries contain “维他命”"; **Show** lists a few of them. When some entries kept the word in their final text, the card warns that it may be correct there; it also warns about one-character forms.
- When the right spelling is already an entry, the card merges the heard-as form into it and the button reads **Add to “Voltip”**. When the form belongs to another entry, the card says so and cannot save.
- The top of the card switches to **Rule**: find and replace side by side, with a preview of the rule on this entry's final text.

In the **Compare** view, selecting a pair of changes AI polish made first asks which spelling is right: the polished one adds the original form to the dictionary; the original one (AI polish changed it wrongly) adds it as the right spelling, so AI polish keeps it from then on.

After saving, the bottom of the window shows "Added to the dictionary · Voltip"; **Undo** or Ctrl Z within 5 seconds takes it back. Saved history entries do not change; the dictionary and the rules apply to later dictations.

On the phone, choose **Fix recognition** in an entry: the original text shows word by word; tap a word to select it, tap another to extend the selection, then choose **Add to dictionary** or **New rule** at the bottom.

To let AI tools such as Claude Code or Codex analyse the history with your projects in mind and suggest entries, see [AI tools](/recognition/mcp).

## Suggestions

**Suggestions**, at the top of the Dictionary page, lists the spellings that were corrected in your recent history: what was recognised, the right spelling, how many times and when last, the most frequent first. The corrections come from two sources, and each row names its source:

- **Polish**: spellings AI polish corrected.
- **Your correction**: spellings you corrected in another app and gave to Voltip with [Learn from selection](#learn-from-selection). Errors that neither recognition nor polish got right can be learned this way.

Spellings the dictionary already corrects and pairs you ignored are left out. Voltip compares the recognised text, the polished text and your corrected text character by character; no AI is used and nothing is sent anywhere.

<ScreenFigure src="/screens/dictionary-suggestions-en-light.webp" width="1440" height="900"
  alt="Suggestions on the Dictionary page, showing the five Suggested pairs, each marked with its source, Polish or Your correction: 维他命 → Voltip has both, and Quinn for → Qwen 4 was seen once, from your correction. The first two were added to the dictionary together: the line above the list says “Added 2 pairs to the dictionary.” and has Undo. The pairs for 瑞迪斯 and 泰普斯克瑞普特 are checked, and the bar above the list adds or ignores them at once. Under the explanation, a line says to leave the cursor in the change or select the corrected text and press the Learn from selection shortcut, and offers to set it. The line under the list says 2 entries have your corrections, 5 pairs are shown and 4 pairs seen once are hidden."
  caption="Each pair names its source; check several rows to add them to the dictionary or ignore them at once, and Undo takes back the whole batch." />

- By default the list shows the **Suggested** pairs: those seen twice or more, and those you corrected (even once). Choose **All** above the list to include the other pairs seen once. The number after **Suggestions** counts the pairs left to decide in the current view, and the line under the list says how many pairs are shown, how many entries carry your corrections, and how many pairs seen once are hidden.
- **Add to dictionary** adds the recognised spelling as a heard-as form: to the entry that already has the right spelling, or to a new entry. When a replacement rule would change the right spelling, or the spelling belongs to another entry, Voltip says why and saves nothing.
- **Ignore** keeps the pair off the list and moves it to **Ignored** below.
- Check several rows, or **Select all** in the header, to add them all to the dictionary or ignore them at once. Pairs with the same right spelling go into one entry; when a pair cannot be added, the reason is under its row and the others are added.
- **Ignore all** ignores every pair the list shows, and the button says how many.
- After adding or ignoring, the row stays where it was, and **Undo** takes it back; when several pairs were handled at once, the result is shown above the list and its **Undo** takes back the whole batch. Once you leave the dictionary page, adding can no longer be undone; an ignored pair can be brought back from **Ignored**.
- Expand a row to see the recognised text it appeared in, and how many of your recent entries adding it would change. Voltip warns when the spelling is a single character, which can change other words by mistake.
- **Ignored (n)** lists the pairs you ignored, the most recent first, with when you ignored each. **Stop ignoring** puts a pair back among the suggestions. The list stays after you leave the page or restart Voltip.

Suggestions come from the history: while history is off, the view only says that history needs to be on; imported audio or video files are not compared, and a dictation with neither AI polish nor a correction of yours has nothing to compare. Chinese is compared character by character; when only one character changed, the pair listed takes the character next to it as well: when 非书 was corrected to 飞书, the pair listed is 非书 → 飞书. Suggestions are available on the computer only.

## Learn from selection

After you correct a dictation in another app, you can give Voltip the corrected text: it notes the change and adds it to the suggestions.

1. Under **Settings → Shortcuts**, set a shortcut for **Learn from selection**. It is not set by default, and it must differ from the shortcuts for dictation, voice editing and copying the last result.
2. When the inserted text has an error, correct it where it is, for example “I'm testing Quinn for max” to “I'm testing Qwen 4 max”.
3. Leave the cursor in the change, or select the corrected text, and press the shortcut. Selecting only the corrected words, the whole sentence, or several dictations in a row all work.
4. The overlay shows “Noted 1 change: Quinn for → Qwen 4”. **Add to dictionary** adds it at once, and you can undo it; **View** opens the suggestions. You can also leave it: the change stays among the suggestions, marked **Your correction**.

- Voltip reads the selection by copying it, and puts the clipboard back afterwards. Through the system's accessibility interface it also reads the field and keeps up to 200 characters before and after the cursor, within its paragraph, to find the dictation (how it reads: [Privacy](/privacy#learn-from-selection)). In a terminal it reads nothing and presses no key; password fields, remote desktops and virtual machines are not read through the accessibility interface, only the selection is copied.
- Voltip looks for the dictation at the cursor among your last 50 dictations; other dictations in the same paragraph are left alone. When the field cannot be read (some apps cannot be, or the read takes too long), only the selection is compared: then select the unchanged words around your change too.
- When none matches, the selection is the same as what was dictated, or nothing is selected and nothing near the cursor can be read, the overlay says so.
- The corrected text is kept with that history entry: its details gain a **Later corrected to** line, and **Remove** deletes it. Learn from selection needs the history to be on.
- When a change has a spelling of one character or one or two letters, the overlay offers **View** only: check what it would change on the dictionary page before adding it.
- The overlay stays for 8 seconds, counted again after each button, and goes when the next dictation starts. While a dictation's result card is on screen, it appears after the card is closed.
- In a pure Wayland session, bind a shortcut to the command `voltip-desktop --learn-selection` in the system settings.
- Learn from selection is available on the computer only.

## Simplified and Traditional Chinese

Under **Chinese script** on the Speech models page's **Recognition** tab, choose **Simplified** (default), **Traditional** or **As recognised**. The script is converted right after recognition, so the dictionary matches the characters you chose.
