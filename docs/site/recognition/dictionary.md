---
description: The personal dictionary corrects words the recogniser gets wrong; replacement rules rewrite the final text; both run on your computer.
---

# Dictionary and rules

Two lists on the sidebar change the recognised text before it is inserted. Both run on your computer, and if either fails for any reason the dictation continues with the text as it was.

## Dictionary

The **Dictionary** page lists words the recogniser tends to get wrong. Each entry has:

- the **right spelling**, for example `kubectl` or a colleague's name;
- up to 10 spellings it is **heard as**, for example `cube control`.

Right after recognition, every heard-as spelling in the text is replaced with the right one, before AI polish and the rules. The right spellings of enabled entries are also sent as a hint to cloud recognition and to AI polish, which keep them as written. Local models take no hints, so for them the dictionary works through the corrections alone.

Matching works like this:

- Letters are matched regardless of case.
- In languages written with spaces, only whole words match: `cat` is not replaced inside `concatenate`.
- Chinese, Japanese and Korean text is matched anywhere in the text.
- The longest match wins, and a replaced word is not matched again.

The **Try it** box on the page shows what the dictionary changes in a sample text; **Use the last dictation** fills it with your latest recording. You can also correct a misheard word right in the history, see [Correct from the history](#correct-from-the-history).

<ScreenFigure src="/screens/dictionary-en-light.webp" width="1440" height="900"
  alt="The Dictionary page with three entries, and the Try it box showing a sample sentence before and after the corrections."
  caption="Three entries, and their corrections in the Try it box." />

The dictionary holds up to 500 entries. Their order is their priority when the hint to cloud services has to be shortened.

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

## Correct from the history

Open an entry in **History** and switch to the **Original** view: it shows the text as the recogniser wrote it, which is where misheard words are.

- Words AI polish already corrected have a dotted blue underline; hovering shows the polished spelling, for example "Polished as 「Voltip」". Selecting one opens a prefilled dictionary card.
- Words that are already the heard-as form of an entry have a solid green underline.
- Select any other misheard word and a toolbar appears above it: **Add to dictionary** (D) or **New rule** (R). Double-click selects a word. When the selection is longer than 64 characters or spans lines, **Add to dictionary** is unavailable and says why.

The card opens under the selection, without leaving the page:

- **Heard as** is the selected text; **Right spelling** is prefilled from the polished text where possible, marked **From the polished text**, so Enter saves it.
- The card tells you what the change would affect, for example "Besides this one, 3 of the last 500 entries contain 「维他命」"; **Show** lists a few of them. When some entries kept the word in their final text, the card warns that it may be correct there; it also warns about one-character forms.
- When the right spelling is already an entry, the card merges the heard-as form into it and the button reads **Add to 「Voltip」**. When the form belongs to another entry, the card says so and cannot save.
- The top of the card switches to **Rule**: find and replace side by side, with a preview of the rule on this entry's final text.

In the **Compare** view, selecting a pair of changes first asks which spelling is right: the polished one adds the original form to the dictionary; the original one (AI polish changed it wrongly) adds it as the right spelling, so AI polish keeps it from then on.

After saving, the bottom of the window shows "Added to the dictionary · Voltip"; **Undo** or Ctrl Z within 5 seconds takes it back. Saved history entries do not change; the dictionary and the rules apply to later dictations.

On the phone, choose **Fix recognition** in an entry: the original text shows word by word; tap a word to select it, tap another to extend the selection, then choose **Add to dictionary** or **New rule** at the bottom.

To let AI tools such as Claude Code or Codex analyse the history with your projects in mind and suggest entries, see [AI tools](/recognition/mcp).

## Simplified and Traditional Chinese

Under **Chinese script** on the Speech models page's **Recognition** tab, choose **Simplified** (default), **Traditional** or **As recognised**. The script is converted right after recognition, so the dictionary matches the characters you chose.
