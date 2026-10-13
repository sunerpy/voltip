// 纠正识别错误 on the phone (docs/dictation.md §16.9 4, boards 12 and 13): the original text as
// tappable words, the bar with the selection and its two actions, and the bottom sheet that writes a
// dictionary entry or a replacement rule with the desktop card's fields, status rows and notes
// (apps/desktop/src/pages/history/QuickFixCard.tsx). Both shells work from the same pure functions
// (`@voltip/shared`'s quick-fix.ts). The sheet opens without the keyboard: its fields are usually
// filled already, and only need confirming.
import {
  type BacktestDraft,
  type BacktestExample,
  type FixKind,
  type FixPrefill,
  type FixStatus,
  type FixStatusLine,
  type FixTarget,
  HEARD_AS_JOINER,
  type HistoryEntry,
  MAX_TERM_CHARS,
  NIL_ID,
  type RuleKind,
  type TextMark,
  type TextRange,
  type UiState,
  type WordPiece,
  errorText,
  excerptSegments,
  fixStatus,
  fixStatusLines,
  fixStatusTone,
  formatDateTime,
  planDictionary,
  planRule,
  ruleNameFor,
  shortClockLabel,
  shortForm,
  snippetParts,
  splitHeardAs,
} from "@voltip/shared";
import { type ComponentProps, memo, useMemo, useState } from "react";
import { Pressable, ScrollView, StyleSheet, View } from "react-native";
import {
  ActivityIndicator,
  Icon,
  IconButton,
  Switch,
  Text,
  TextInput,
  TouchableRipple,
} from "react-native-paper";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import Svg, { Line } from "react-native-svg";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useVocabularyBacktest } from "../hooks/useVocabularyBacktest";
import { useVocabularyPreview } from "../hooks/useVocabularyPreview";
import type { AppTheme } from "../theme/themes";
import { Button } from "../ui/Button";
import { Hint, Section, Segmented, cardShadow, styles as kit, useAppTheme } from "../ui/kit";
import { BottomSheet, sheetColor } from "../ui/Sheet";

// ---- the words ----------------------------------------------------------------------------------

/** What the words view lays out: a word with the punctuation stuck to it (an opening bracket or
 *  quote before it, the rest after it), or a line break. White space between words is the gap. A
 *  unit without a word is punctuation on its own (at the start, or after a space). */
export type WordUnit =
  | { kind: "unit"; key: number; pieces: WordPiece[]; word: WordPiece | undefined }
  | { kind: "break"; key: number };

const OPENING = /^[\p{Ps}\p{Pi}]+$/u;
const SPACE = /^\s+$/u;
/** The selection ring's width: every word keeps the room for it, so a tap moves nothing. */
const CHIP_BORDER = 1.5;

export function wordUnits(pieces: readonly WordPiece[]): WordUnit[] {
  const out: WordUnit[] = [];
  // The unit the next piece may join: the last one, until white space ends it.
  let open: Extract<WordUnit, { kind: "unit" }> | undefined;
  for (const piece of pieces) {
    if (SPACE.test(piece.text)) {
      if (piece.text.includes("\n")) out.push({ kind: "break", key: piece.start });
      open = undefined;
      continue;
    }
    const last = open?.pieces.at(-1);
    // A word joins an opening bracket waiting for it, an opening bracket joins punctuation that has
    // no word yet, and other punctuation sticks to the unit before it.
    const joins =
      open !== undefined &&
      last !== undefined &&
      (piece.word
        ? open.word === undefined && OPENING.test(last.text)
        : !OPENING.test(piece.text) || open.word === undefined);
    if (open !== undefined && joins) {
      open.pieces.push(piece);
      if (piece.word) open.word = piece;
    } else {
      open = {
        kind: "unit",
        key: piece.start,
        pieces: [piece],
        word: piece.word ? piece : undefined,
      };
      out.push(open);
    }
  }
  return out;
}

/** `range` lies in `selection`. */
function within(selection: TextRange | undefined, range: TextRange): boolean {
  return selection !== undefined && selection.start <= range.start && range.end <= selection.end;
}

/** The underline that says a word can be tapped (faint), a replacement the polish made (dotted, in
 *  the accent) or a form the dictionary corrects (solid, in the ok colour). */
function Underline({ mark, theme }: { mark: TextMark | undefined; theme: AppTheme }) {
  if (mark?.kind === "polish") {
    // A dotted border on one side alone does not draw on every Android version: dots in an SVG do.
    return (
      <View style={[styles.underline, { height: 2 }]} testID="fix-underline-polish">
        <Svg width="100%" height={2}>
          <Line
            x1="1"
            y1="1"
            x2="100%"
            y2="1"
            stroke={theme.colors.primary}
            strokeWidth={2}
            strokeDasharray="2 2"
          />
        </Svg>
      </View>
    );
  }
  return (
    <View
      style={[
        styles.underline,
        mark?.kind === "known"
          ? { height: 1.5, backgroundColor: theme.voltip.okText }
          : { height: 1, backgroundColor: theme.colors.outlineVariant },
      ]}
    />
  );
}

interface UnitProps {
  unit: Extract<WordUnit, { kind: "unit" }>;
  /** One `1` / `0` per piece: inside the selection or not (by value, so a tap redraws only the
   *  units it changes). */
  lit: string;
  label: string;
  theme: AppTheme;
  onTap: (piece: WordPiece) => void;
}

const Unit = memo(function Unit({ unit, lit, label, theme, onTap }: UnitProps) {
  const word = unit.word;
  const chosen = word !== undefined && lit[unit.pieces.indexOf(word)] === "1";
  const body = unit.pieces.map((piece, i) => {
    const on = lit[i] === "1";
    if (piece !== word) {
      return (
        <Text
          key={piece.start}
          style={[
            styles.word,
            styles.punctuation,
            { color: on ? theme.colors.primary : theme.colors.onSurface },
          ]}>
          {piece.text}
        </Text>
      );
    }
    return (
      <View
        key={piece.start}
        style={[
          styles.chip,
          chosen && {
            borderColor: theme.colors.primary,
            backgroundColor: theme.colors.primaryContainer,
          },
        ]}>
        <Text
          style={[styles.word, { color: chosen ? theme.colors.primary : theme.colors.onSurface }]}>
          {piece.text}
        </Text>
        {!chosen && <Underline mark={piece.mark} theme={theme} />}
      </View>
    );
  });
  if (word === undefined) return <View style={styles.unit}>{body}</View>;
  return (
    <Pressable
      testID={`fix-word-${word.start}`}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ selected: chosen }}
      onPress={() => {
        onTap(word);
      }}
      style={styles.unit}>
      {body}
    </Pressable>
  );
});

/** The original text as words to tap (board 12): each a target with a faint underline, the marks
 *  whole, the selection in a ring. */
export function FixWords({
  pieces,
  selection,
  onTap,
}: {
  pieces: readonly WordPiece[];
  selection: TextRange | undefined;
  onTap: (piece: WordPiece) => void;
}) {
  const theme = useAppTheme();
  const { t } = useI18n();
  const units = useMemo(() => wordUnits(pieces), [pieces]);
  const label = (piece: WordPiece) => {
    const mark = piece.mark;
    if (mark?.kind === "polish" && mark.pair !== undefined)
      return t("history.fix.marks.polishedLabel", {
        heard: mark.text,
        polished: mark.pair.polished,
      });
    if (mark?.kind === "known" && mark.entry !== undefined)
      return t("history.fix.marks.knownLabel", { heard: mark.text, term: mark.entry.term });
    return piece.text;
  };
  return (
    <Section padded testID="fix-words">
      <View style={styles.words}>
        {units.map((unit) =>
          unit.kind === "break" ? (
            <View key={`b${unit.key}`} style={styles.lineBreak} />
          ) : (
            <Unit
              key={unit.key}
              unit={unit}
              lit={unit.pieces.map((p) => (within(selection, p) ? "1" : "0")).join("")}
              label={unit.word === undefined ? "" : label(unit.word)}
              theme={theme}
              onTap={onTap}
            />
          ),
        )}
      </View>
    </Section>
  );
}

/** Under the words: how to pick them, and what the marks are (board 12). */
export function FixLegend({ marks }: { marks: readonly TextMark[] }) {
  const theme = useAppTheme();
  const { t } = useI18n();
  const polished = marks.filter((m) => m.kind === "polish").length;
  const known = marks.length - polished;
  const muted = { color: theme.colors.onSurfaceVariant };
  return (
    <View style={{ gap: 6 }}>
      <Hint>{t("mobile.entry.fixHint")}</Hint>
      {marks.length > 0 && (
        <View style={styles.legend} testID="fix-legend">
          {polished > 0 && (
            <View style={styles.legendItem}>
              <Svg width={18} height={2}>
                <Line
                  x1="1"
                  y1="1"
                  x2="18"
                  y2="1"
                  stroke={theme.colors.primary}
                  strokeWidth={2}
                  strokeDasharray="2 2"
                />
              </Svg>
              <Text variant="bodySmall" style={muted}>
                {t("history.fix.marks.polishedCount", { n: polished })}
              </Text>
            </View>
          )}
          {known > 0 && (
            <View style={styles.legendItem}>
              <View style={{ width: 16, height: 1.5, backgroundColor: theme.voltip.okText }} />
              <Text variant="bodySmall" style={muted}>
                {t("history.fix.marks.knownCount", { n: known })}
              </Text>
            </View>
          )}
          <Text variant="bodySmall" style={{ color: theme.voltip.subtle }}>
            {`· ${t("mobile.entry.fixMarkHint")}`}
          </Text>
        </View>
      )}
    </View>
  );
}

/** The height the bar takes at the bottom of the screen, so the words scroll clear of it. */
export const FIX_BAR_SPACE = 176;

/** The selection and what can be made of it, at the bottom of the screen (board 12): 加入词典 and
 *  新建规则, 48 dp tall; the default one filled, 加入词典 off with the reason when the words cannot
 *  be a dictionary form. */
export function FixBar({
  target,
  onClear,
  onOpen,
}: {
  target: FixTarget;
  onClear: () => void;
  onOpen: (kind: FixKind) => void;
}) {
  const theme = useAppTheme();
  const { t } = useI18n();
  const insets = useSafeAreaInsets();
  const block =
    target.block === "tooLong"
      ? t("history.fix.toolbar.tooLong", { max: MAX_TERM_CHARS })
      : target.block === "oneLine"
        ? t("history.fix.toolbar.oneLine")
        : undefined;
  const ruleFirst = target.kind === "rule";
  return (
    <View
      testID="fix-bar"
      style={[
        styles.bar,
        {
          bottom: insets.bottom + 12,
          backgroundColor: sheetColor(theme),
          borderColor: theme.voltip.cardEdge,
        },
        cardShadow(theme, 12, 32, 0.14),
      ]}>
      <View style={{ flexDirection: "row", alignItems: "flex-start", gap: 8 }}>
        <View style={{ flex: 1, gap: 2, paddingTop: 4 }}>
          <Text
            variant="titleSmall"
            numberOfLines={2}
            testID="fix-bar-text"
            style={{ fontWeight: "600" }}>
            {t("mobile.entry.fixSelected", { text: target.text })}
          </Text>
          {target.pair !== undefined && (
            <Text
              variant="bodySmall"
              numberOfLines={1}
              style={{ color: theme.colors.onSurfaceVariant }}>
              {t("mobile.entry.fixPolished", { polished: target.pair.polished })}
            </Text>
          )}
          {block !== undefined && (
            <Text
              variant="bodySmall"
              testID="fix-bar-block"
              style={{ color: theme.voltip.warning }}>
              {block}
            </Text>
          )}
        </View>
        <IconButton
          icon="close"
          mode="contained-tonal"
          size={18}
          accessibilityLabel={t("mobile.entry.fixClear")}
          testID="fix-bar-clear"
          onPress={onClear}
          style={{ margin: 0 }}
        />
      </View>
      <View style={{ flexDirection: "row", gap: 12 }}>
        <Button
          mode={ruleFirst ? "outlined" : "contained"}
          icon="book-outline"
          disabled={block !== undefined}
          testID="fix-bar-dictionary"
          style={{ flex: 1 }}
          contentStyle={styles.tall}
          onPress={() => {
            onOpen("dictionary");
          }}>
          {t("history.fix.toolbar.dictionary")}
        </Button>
        <Button
          mode={ruleFirst ? "contained" : "outlined"}
          icon="creation"
          testID="fix-bar-rule"
          style={{ flex: 1 }}
          contentStyle={styles.tall}
          onPress={() => {
            onOpen("rule");
          }}>
          {t("history.fix.toolbar.rule")}
        </Button>
      </View>
    </View>
  );
}

// ---- the sheet ----------------------------------------------------------------------------------

/** What the sheet opens with; a new `id` for every opening, so the form starts from its prefill. */
export interface FixOpen {
  id: number;
  kind: FixKind;
  prefill: FixPrefill;
}

/** A save the toast can undo: `undo` gets the lists as they are when it is pressed. */
export interface SavedFix {
  message: string;
  undo: (latest: UiState) => Promise<void>;
}

/** The quick fix's bottom sheet (board 13): the same component as the pickers' sheets. */
export function FixSheet({
  open,
  spec,
  entry,
  onClose,
  onSaved,
  onOpenDictionary,
}: {
  open: boolean;
  spec: FixOpen | undefined;
  entry: HistoryEntry;
  onClose: () => void;
  onSaved: (saved: SavedFix) => void;
  onOpenDictionary: () => void;
}) {
  return (
    <BottomSheet open={open} close={onClose} maxHeight={0.9} testID="fix-sheet">
      {spec !== undefined && (
        <FixForm
          key={spec.id}
          spec={spec}
          entry={entry}
          onClose={onClose}
          onSaved={onSaved}
          onOpenDictionary={onOpenDictionary}
        />
      )}
    </BottomSheet>
  );
}

function FixForm({
  spec,
  entry,
  onClose,
  onSaved,
  onOpenDictionary,
}: {
  spec: FixOpen;
  entry: HistoryEntry;
  onClose: () => void;
  onSaved: (saved: SavedFix) => void;
  onOpenDictionary: () => void;
}) {
  const theme = useAppTheme();
  const { t } = useI18n();
  const { backend } = useBackend();
  const state = useUiState();
  const initial = spec.prefill;
  const [kind, setKind] = useState<FixKind>(spec.kind);
  const [heard, setHeard] = useState(initial.heard);
  const [term, setTerm] = useState(initial.term);
  const [find, setFind] = useState(initial.heard);
  const [replace, setReplace] = useState(initial.term);
  const [name, setName] = useState<string | undefined>(undefined);
  const [ruleKind, setRuleKind] = useState<RuleKind>("literal");
  // The rules page's default for a new rule (§16.9 3.4).
  const [caseSensitive, setCaseSensitive] = useState(true);
  const [saveError, setSaveError] = useState<string | undefined>(undefined);
  const [expanded, setExpanded] = useState(false);
  const [saving, setSaving] = useState(false);

  const dict = useMemo(
    () => planDictionary(term, heard, state.dictionary, t),
    [term, heard, state.dictionary, t],
  );
  const ruleName = name ?? ruleNameFor(find, replace, state.rules);
  const rule = useMemo(
    () =>
      planRule({ find, replace, name: ruleName, kind: ruleKind, caseSensitive }, state.rules, t),
    [find, replace, ruleName, ruleKind, caseSensitive, state.rules, t],
  );
  // The rule as the core sees it (a regex in the Rust dialect), on this entry's final text.
  const previewDraft = useMemo(
    () => (kind === "rule" && rule.ready ? { id: null, rule: rule.draft } : undefined),
    [kind, rule],
  );
  const preview = useVocabularyPreview(previewDraft === undefined ? "" : entry.text, previewDraft);
  const previewError = preview.kind === "error" ? preview.message : undefined;
  const draft: BacktestDraft | undefined =
    kind === "dictionary"
      ? dict.ready && dict.added.length > 0
        ? { kind: "dictionary", id: dict.merge?.id ?? null, entry: dict.draft }
        : undefined
      : rule.ready && previewError === undefined
        ? { kind: "rule", id: null, rule: rule.draft }
        : undefined;
  const backtest = useVocabularyBacktest(draft, entry.id);
  const forms = kind === "dictionary" ? dict.added : find.trim() === "" ? [] : [find];
  const shownForms = forms.join(HEARD_AS_JOINER);
  const rows = fixStatus({
    conflict: kind === "dictionary" ? dict.conflict : undefined,
    error: previewError ?? (backtest.kind === "error" ? backtest.message : undefined),
    merge: kind === "dictionary" ? dict.merge : undefined,
    heard: shownForms,
    backtest:
      backtest.kind === "computing"
        ? "computing"
        : backtest.kind === "ok"
          ? backtest.backtest
          : undefined,
    short: shortForm(forms),
  });
  const examples = backtest.kind === "ok" ? backtest.backtest.examples : [];
  const ready =
    !saving && (kind === "dictionary" ? dict.ready : rule.ready && previewError === undefined);

  const edited = () => {
    setSaveError(undefined);
  };
  const switchKind = (next: FixKind) => {
    // The words carry over: 曾听成 is what a rule finds, 正确写法 what it writes.
    if (next === "rule") {
      setFind(splitHeardAs(heard)[0] ?? "");
      setReplace(term);
    } else {
      setHeard(find);
      setTerm(replace);
    }
    setKind(next);
    setExpanded(false);
    edited();
  };

  const save = async () => {
    if (!ready) return;
    setSaving(true);
    try {
      if (kind === "dictionary") {
        const merge = dict.merge;
        if (merge !== undefined) {
          await backend.invoke("dictionary_update", { id: merge.id, entry: dict.draft });
          const before = { term: merge.term, heard_as: merge.heard_as, enabled: merge.enabled };
          onSaved({
            message: t("history.addToDictionary.added", { term: merge.term }),
            undo: async () => {
              await backend.invoke("dictionary_update", { id: merge.id, entry: before });
            },
          });
        } else {
          const known = new Set(state.dictionary.map((e) => e.id));
          const saved = dict.draft.term;
          await backend.invoke("dictionary_add", { entry: dict.draft, historyId: entry.id });
          onSaved({
            message: t("history.addToDictionary.added", { term: saved }),
            undo: async (latest) => {
              const added = latest.dictionary.find((e) => !known.has(e.id) && e.term === saved);
              if (added !== undefined) await backend.invoke("dictionary_remove", { id: added.id });
            },
          });
        }
      } else {
        const known = new Set(state.rules.map((r) => r.id));
        const saved = rule.draft.name;
        await backend.invoke("rules_add", { rule: rule.draft });
        onSaved({
          message: t("history.fix.done.rule", { name: saved }),
          undo: async (latest) => {
            const added = latest.rules.find((r) => !known.has(r.id) && r.name === saved);
            if (added !== undefined) await backend.invoke("rules_remove", { id: added.id });
          },
        });
      }
    } catch (e) {
      // The core refused the draft: the sheet stays with its words and says why (§16.9 3.3).
      setSaveError(errorText(e));
      setSaving(false);
    }
  };

  const title =
    kind === "dictionary" ? t("history.addToDictionary.title") : t("history.fix.card.ruleTitle");
  const primary =
    kind === "rule"
      ? t("history.fix.card.saveRule")
      : dict.merge !== undefined
        ? t("history.fix.card.addTo", { term: dict.merge.term })
        : t("history.addToDictionary.button");
  const field = [kit.mono, { backgroundColor: sheetColor(theme) }];

  return (
    <>
      <ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={styles.form}>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
          <Text
            variant="titleLarge"
            accessibilityRole="header"
            numberOfLines={1}
            style={{ flex: 1, fontWeight: "700" }}>
            {title}
          </Text>
          <View style={{ width: 200 }}>
            <Segmented
              label={t("history.fix.card.kindLabel")}
              value={kind}
              onChange={switchKind}
              options={[
                {
                  value: "dictionary",
                  label: t("history.fix.card.kindDictionary"),
                  testID: "fix-kind-dictionary",
                },
                { value: "rule", label: t("history.fix.card.kindRule"), testID: "fix-kind-rule" },
              ]}
            />
          </View>
        </View>

        {kind === "dictionary" ? (
          <>
            <View style={{ gap: 4 }}>
              <TextInput
                mode="outlined"
                label={t("history.addToDictionary.heard")}
                value={heard}
                autoCapitalize="none"
                autoCorrect={false}
                error={dict.problem?.heard !== undefined}
                testID="fix-heard"
                style={field}
                onChangeText={(v) => {
                  setHeard(v);
                  edited();
                }}
              />
              {dict.problem?.heard !== undefined && <Hint tone="danger">{dict.problem.heard}</Hint>}
            </View>
            <TermField
              value={term}
              prefill={initial}
              error={saveError ?? (term.trim() === "" ? undefined : dict.problem?.term)}
              style={field}
              onChange={(v) => {
                setTerm(v);
                edited();
              }}
            />
          </>
        ) : (
          <>
            <View style={{ gap: 4 }}>
              <TextInput
                mode="outlined"
                label={t("history.fix.card.find")}
                value={find}
                autoCapitalize="none"
                autoCorrect={false}
                error={saveError !== undefined}
                testID="fix-find"
                style={field}
                onChangeText={(v) => {
                  setFind(v);
                  edited();
                }}
              />
              {saveError !== undefined && <Hint tone="danger">{saveError}</Hint>}
            </View>
            <TextInput
              mode="outlined"
              label={t("history.fix.card.replace")}
              value={replace}
              autoCapitalize="none"
              autoCorrect={false}
              testID="fix-replace"
              style={field}
              onChangeText={(v) => {
                setReplace(v);
                edited();
              }}
            />
            <Segmented
              label={t("rules.editor.kind")}
              value={ruleKind}
              onChange={(v) => {
                setRuleKind(v);
                edited();
              }}
              options={[
                { value: "literal", label: t("rules.kind.literal"), testID: "fix-rule-literal" },
                { value: "regex", label: t("rules.kind.regex"), testID: "fix-rule-regex" },
              ]}
            />
            <TouchableRipple
              accessibilityRole="switch"
              accessibilityLabel={t("rules.editor.caseSensitive")}
              accessibilityState={{ checked: caseSensitive }}
              testID="fix-case"
              onPress={() => {
                setCaseSensitive(!caseSensitive);
                edited();
              }}>
              <View style={styles.switchRow}>
                <Text variant="bodyLarge" style={{ flex: 1 }}>
                  {t("rules.editor.caseSensitive")}
                </Text>
                <View pointerEvents="none">
                  <Switch value={caseSensitive} importantForAccessibility="no-hide-descendants" />
                </View>
              </View>
            </TouchableRipple>
            <View style={{ gap: 4 }}>
              <TextInput
                mode="outlined"
                label={t("history.fix.card.name")}
                value={ruleName}
                error={ruleName.trim() !== "" && rule.problem?.name !== undefined}
                testID="fix-name"
                style={{ backgroundColor: sheetColor(theme) }}
                onChangeText={(v) => {
                  setName(v);
                  edited();
                }}
              />
              {ruleName.trim() !== "" && rule.problem?.name !== undefined && (
                <Hint tone="danger">{rule.problem.name}</Hint>
              )}
            </View>
            {preview.kind === "ok" && (
              <RulePreview
                before={entry.text}
                after={preview.preview.output}
                fired={preview.preview.rules.some((r) => r.id === NIL_ID)}
                pattern={find}
              />
            )}
          </>
        )}

        {rows.length > 0 && (
          <StatusBox
            rows={rows}
            heard={shownForms}
            rule={kind === "rule"}
            examples={examples}
            expanded={expanded}
            onToggle={() => {
              setExpanded((was) => !was);
            }}
            onOpenDictionary={onOpenDictionary}
          />
        )}

        <Text variant="bodySmall" testID="fix-note" style={{ color: theme.voltip.subtle }}>
          {kind === "rule" ? t("history.fix.card.ruleNote") : t("history.fix.card.dictionaryNote")}
        </Text>
      </ScrollView>
      <View style={styles.footer}>
        <Button mode="outlined" testID="fix-cancel" contentStyle={styles.tall} onPress={onClose}>
          {t("common.cancel")}
        </Button>
        <Button
          mode="contained"
          disabled={!ready}
          testID="fix-save"
          style={{ flex: 1 }}
          contentStyle={styles.tall}
          onPress={() => void save()}>
          {primary}
        </Button>
      </View>
    </>
  );
}

/** 正确写法 and where its words came from (来自润色结果), while they are unchanged. */
function TermField({
  value,
  prefill,
  error,
  style,
  onChange,
}: {
  value: string;
  prefill: FixPrefill;
  error: string | undefined;
  style: ComponentStyle;
  onChange: (v: string) => void;
}) {
  const theme = useAppTheme();
  const { t } = useI18n();
  const from = prefill.termFrom !== undefined && value === prefill.term && value !== "";
  return (
    <View style={{ gap: 4 }}>
      <TextInput
        mode="outlined"
        label={t("history.addToDictionary.term")}
        value={value}
        autoCapitalize="none"
        autoCorrect={false}
        error={error !== undefined}
        testID="fix-term"
        style={style}
        onChangeText={onChange}
      />
      {from && (
        <View style={styles.tag} testID="fix-term-from">
          <Icon
            source={prefill.termFrom === "polish" ? "auto-fix" : "check"}
            size={14}
            color={theme.colors.primary}
          />
          <Text variant="labelMedium" style={{ color: theme.colors.primary }}>
            {prefill.termFrom === "polish"
              ? t("history.fix.card.fromPolish")
              : t("history.fix.card.fromRaw")}
          </Text>
        </View>
      )}
      {error !== undefined && <Hint tone="danger">{error}</Hint>}
    </View>
  );
}

type ComponentStyle = ComponentProps<typeof TextInput>["style"];

/** 本条预览: this entry's final text with the draft applied, cut to the changes (§16.9 3.4). */
function RulePreview({
  before,
  after,
  fired,
  pattern,
}: {
  before: string;
  after: string;
  fired: boolean;
  pattern: string;
}) {
  const theme = useAppTheme();
  const { t } = useI18n();
  const segments = fired ? excerptSegments(before, after) : [];
  return (
    <View style={{ gap: 6 }}>
      <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
        {t("history.fix.card.preview")}
      </Text>
      <View style={[styles.box, { backgroundColor: theme.voltip.inset }]} testID="fix-preview">
        {segments.length === 0 ? (
          <Text variant="bodyMedium" style={{ color: theme.colors.onSurfaceVariant }}>
            {t("history.fix.card.previewNone", { pattern })}
          </Text>
        ) : (
          <Text variant="bodyMedium">
            {segments.map((seg, i) =>
              seg.kind === "same" ? (
                <Text key={i}>{seg.text}</Text>
              ) : (
                <Text
                  key={i}
                  style={
                    seg.kind === "del"
                      ? {
                          color: theme.voltip.danger,
                          backgroundColor: theme.voltip.dangerSoft,
                          textDecorationLine: "line-through",
                        }
                      : { color: theme.voltip.okText, backgroundColor: theme.voltip.okSoft }
                  }>
                  {seg.text}
                </Text>
              ),
            )}
          </Text>
        )}
      </View>
    </View>
  );
}

/** The status rows in their box, worst tone first (§16.9 3.3; board 13). */
function StatusBox({
  rows,
  heard,
  rule,
  examples,
  expanded,
  onToggle,
  onOpenDictionary,
}: {
  rows: readonly FixStatus[];
  heard: string;
  rule: boolean;
  examples: readonly BacktestExample[];
  expanded: boolean;
  onToggle: () => void;
  onOpenDictionary: () => void;
}) {
  const theme = useAppTheme();
  const { t } = useI18n();
  const lines = fixStatusLines(rows, { heard, rule, examples: examples.length }, t);
  const worst = fixStatusTone(lines);
  const linkLabel = (link: NonNullable<FixStatusLine["link"]>) =>
    link === "show"
      ? expanded
        ? t("common.collapse")
        : t("history.fix.card.view")
      : link === "openDictionary"
        ? t("history.fix.card.openDictionary")
        : t("history.fix.card.viewEntry");
  return (
    <View
      testID="fix-status"
      accessibilityLiveRegion="polite"
      style={[
        styles.box,
        {
          gap: 8,
          backgroundColor:
            worst === "error"
              ? theme.voltip.dangerSoft
              : worst === "warn"
                ? theme.voltip.warningSoft
                : theme.voltip.inset,
        },
      ]}>
      {lines.map((line, i) => {
        const color =
          line.tone === "error"
            ? theme.voltip.danger
            : line.tone === "warn"
              ? theme.voltip.warning
              : theme.colors.onSurfaceVariant;
        const link = line.link;
        return (
          <View key={i} style={styles.statusRow} testID="fix-status-row">
            {line.tone === "busy" ? (
              <ActivityIndicator size={14} style={{ marginTop: 2 }} />
            ) : (
              <Icon
                source={line.tone === "info" ? "information-outline" : "alert-outline"}
                size={16}
                color={color}
              />
            )}
            <Text
              variant="bodySmall"
              testID="fix-status-text"
              style={{
                flex: 1,
                lineHeight: 18,
                color:
                  line.tone === "info" || line.tone === "busy" ? theme.colors.onSurface : color,
              }}>
              {line.text}
              {link !== undefined && (
                <>
                  {" "}
                  <Text
                    accessibilityRole="link"
                    testID={`fix-status-${link}`}
                    style={{ color: theme.colors.primary, fontWeight: "600" }}
                    onPress={link === "show" ? onToggle : onOpenDictionary}>
                    {linkLabel(link)}
                  </Text>
                </>
              )}
            </Text>
          </View>
        );
      })}
      {expanded && examples.length > 0 && <Examples examples={examples} />}
    </View>
  );
}

/** Up to three takes the draft would change: when, and the words around the change, the changed
 *  words lit (§16.9 3.3). */
function Examples({ examples }: { examples: readonly BacktestExample[] }) {
  const theme = useAppTheme();
  const { locale } = useI18n();
  return (
    <View testID="fix-examples" style={{ marginLeft: 26 }}>
      {examples.map((e) => {
        const [lead, hit, tail] = snippetParts(e.before, e.after);
        const when = `${formatDateTime(locale, e.at_ms, {
          month: locale === "zh-CN" ? "long" : "short",
          day: "numeric",
        })} ${shortClockLabel(e.at_ms)}`;
        return (
          <View
            key={e.history_id}
            style={{
              borderTopWidth: StyleSheet.hairlineWidth,
              borderTopColor: theme.colors.outlineVariant,
              paddingVertical: 6,
            }}>
            <Text variant="labelSmall" style={{ color: theme.voltip.subtle }}>
              {when}
            </Text>
            <Text variant="bodySmall">
              {lead}
              <Text style={{ color: theme.colors.primary, fontWeight: "600" }}>{hit}</Text>
              {tail}
            </Text>
          </View>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  // Top-aligned, the punctuation as far down as a chip's border: one baseline for the line.
  words: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "flex-start",
    columnGap: 5,
    rowGap: 6,
  },
  lineBreak: { flexBasis: "100%", height: 0 },
  unit: { flexDirection: "row", alignItems: "flex-start" },
  punctuation: { marginTop: CHIP_BORDER },
  chip: {
    borderWidth: CHIP_BORDER,
    borderColor: "transparent",
    borderRadius: 8,
    paddingHorizontal: 3,
    paddingBottom: 4,
  },
  word: { fontSize: 17, lineHeight: 28 },
  underline: { position: "absolute", left: 3, right: 3, bottom: 2 },
  legend: { flexDirection: "row", flexWrap: "wrap", alignItems: "center", columnGap: 8, rowGap: 4 },
  legendItem: { flexDirection: "row", alignItems: "center", gap: 6 },
  bar: {
    position: "absolute",
    left: 12,
    right: 12,
    borderRadius: 28,
    borderWidth: 1,
    padding: 16,
    paddingTop: 14,
    gap: 14,
  },
  tall: { height: 48 },
  form: { paddingHorizontal: 24, paddingTop: 4, paddingBottom: 8, gap: 12 },
  footer: { flexDirection: "row", gap: 12, paddingHorizontal: 24, paddingTop: 12 },
  tag: { flexDirection: "row", alignItems: "center", gap: 4, paddingHorizontal: 4 },
  switchRow: { flexDirection: "row", alignItems: "center", gap: 12, minHeight: 48 },
  box: { borderRadius: 10, paddingVertical: 9, paddingHorizontal: 12 },
  statusRow: { flexDirection: "row", alignItems: "flex-start", gap: 10 },
});
