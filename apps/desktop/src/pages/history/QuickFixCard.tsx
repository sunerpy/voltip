// The quick-fix card (docs/dictation.md §16.9 3.3–3.6, boards 03, 04, 05, 10, 11): a non-modal
// dialog under the words that writes a dictionary entry or a replacement rule. 曾听成 comes from the
// selection, 正确写法 from the polished text where the polish replaced the words; the status rows say
// what the draft would change in the recent takes (`vocabulary_backtest`) and what could go wrong;
// the core checks again on save, and a refusal keeps the card open with the reason under the
// fields. The diff view's replacements open it with the question which spelling is right first.
import {
  type BacktestDraft,
  type BacktestExample,
  type DictionaryConflict,
  type FixKind,
  type FixPrefill,
  type FixStatus,
  HEARD_AS_JOINER,
  type HistoryEntry,
  NIL_ID,
  type PolishPair,
  type RightSpelling,
  type RuleKind,
  type TextMark,
  type UiState,
  errorText,
  excerptSegments,
  fixStatus,
  formatDateTime,
  planDictionary,
  planRule,
  prefillPair,
  ruleNameFor,
  shortClockLabel,
  shortForm,
  snippetParts,
  splitHeardAs,
} from "@voltip/shared";
import {
  Button,
  Icon,
  IconButton,
  Input,
  Keycap,
  Segmented,
  Toggle,
  cx,
  useBackend,
  useHitTotals,
  useI18n,
  useUiState,
  useVocabularyPreview,
} from "@voltip/ui";
import {
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { type FloatBox, useFloatingStyle } from "./floating";
import { useBacktest } from "./useBacktest";

/** What a card opens with. */
export type CardOpen =
  | { mode: "fix"; kind: FixKind; prefill: FixPrefill; polishOnly: boolean }
  | { mode: "which"; pair: PolishPair };

/** A save the toast can undo: `undo` gets the lists as they are when it is pressed. */
export interface SavedFix {
  message: string;
  undo: (latest: UiState) => Promise<void>;
}

export interface QuickFixCardProps {
  entry: HistoryEntry;
  open: CardOpen;
  anchor: () => FloatBox | undefined;
  /** Close; `refocus` gives the focus back to what opened the card. */
  onClose: (refocus: boolean) => void;
  /** A field changed: a click elsewhere no longer closes the card. */
  onEdit: () => void;
  onSaved: (saved: SavedFix) => void;
  onOpenDictionary: () => void;
}

/** The card's surface; its width is the caller's (400 px, the diff view's question 420 px). */
const CARD_CLASS =
  "z-40 max-w-[calc(100vw-16px)] overflow-y-auto rounded-[18px] border border-raised-edge bg-raised p-5 text-[13px] text-fg shadow-pop glass-blur";

/** The keycap after a button's label: a hint for the eye, not part of its name. */
function KeyHint({ keys, onPrimary = false }: { keys: string; onPrimary?: boolean }) {
  if (onPrimary) {
    // On the primary fill the keycap is a faint lighter patch (board 03), not the keycap surface.
    return (
      <kbd
        aria-hidden
        className="mono inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-[5px] bg-primary-fg/20 px-1 text-[10px] leading-none text-primary-fg">
        {keys}
      </kbd>
    );
  }
  return (
    <span aria-hidden>
      <Keycap className="h-[18px] px-1 text-[10px]">{keys}</Keycap>
    </span>
  );
}

/** Focus stays in the card: Tab past the last control comes back to the first. */
function trapTab(e: KeyboardEvent<HTMLElement>) {
  if (e.key !== "Tab") return;
  const all = [
    ...e.currentTarget.querySelectorAll<HTMLElement>(
      'button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ),
  ];
  const first = all[0];
  const last = all.at(-1);
  if (first === undefined || last === undefined) return;
  if (e.shiftKey && document.activeElement === first) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && document.activeElement === last) {
    e.preventDefault();
    first.focus();
  }
}

export function QuickFixCard({
  entry,
  open,
  anchor,
  onClose,
  onEdit,
  onSaved,
  onOpenDictionary,
}: QuickFixCardProps) {
  const { t } = useI18n();
  const { backend } = useBackend();
  const state = useUiState();
  const panel = useRef<HTMLDivElement>(null);
  const { style } = useFloatingStyle(panel, anchor, { side: "bottom", align: "start" });
  const pair = open.mode === "which" ? open.pair : undefined;
  const initial = open.mode === "which" ? prefillPair(open.pair, "polished") : open.prefill;
  const [choice, setChoice] = useState<RightSpelling>("polished");
  const [kind, setKind] = useState<FixKind>(open.mode === "which" ? "dictionary" : open.kind);
  const [prefill, setPrefill] = useState<FixPrefill>(initial);
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
  const fieldsId = useId();

  // The right spelling (a rule: what replaces) takes the focus; the selection is in 曾听成.
  useEffect(() => {
    panel.current?.querySelector<HTMLInputElement>(`[data-autofocus="${kind}"]`)?.focus();
  }, [kind]);

  const edited = () => {
    setSaveError(undefined);
    onEdit();
  };

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
  const backtest = useBacktest(draft, entry.id);
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

  const switchKind = (next: FixKind) => {
    if (next === kind) return;
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

  const choose = (next: RightSpelling) => {
    if (pair === undefined) return;
    const filled = prefillPair(pair, next);
    setChoice(next);
    setPrefill(filled);
    setHeard(filled.heard);
    setTerm(filled.term);
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
      // The core refused the draft: the card stays with its words and says why (§16.9 3.3).
      setSaveError(errorText(e));
      setSaving(false);
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose(true);
      return;
    }
    const target = e.target;
    if (
      e.key === "Enter" &&
      target instanceof HTMLInputElement &&
      target.type !== "radio" &&
      !e.nativeEvent.isComposing
    ) {
      e.preventDefault();
      void save();
      return;
    }
    trapTab(e);
  };

  const title =
    open.mode === "which"
      ? t("history.fix.which.title")
      : kind === "dictionary"
        ? t("history.addToDictionary.title")
        : t("history.fix.card.ruleTitle");
  const primary =
    kind === "rule"
      ? t("history.fix.card.saveRule")
      : dict.merge !== undefined
        ? t("history.fix.card.addTo", { term: dict.merge.term })
        : t("history.addToDictionary.button");

  const termTag =
    prefill.termFrom !== undefined && term === prefill.term && term !== "" ? (
      <p
        className="mt-1 flex items-center gap-1 text-[11px] font-medium text-accent-text"
        data-testid="fix-term-from">
        <Icon name={prefill.termFrom === "polish" ? "wand" : "check"} size={12} />
        {prefill.termFrom === "polish"
          ? t("history.fix.card.fromPolish")
          : t("history.fix.card.fromRaw")}
      </p>
    ) : undefined;
  const termField = (
    <div>
      <Input
        label={t("history.addToDictionary.term")}
        mono
        value={term}
        data-autofocus="dictionary"
        error={saveError ?? (term.trim() === "" ? undefined : dict.problem?.term)}
        onChange={(e) => {
          setTerm(e.target.value);
          edited();
        }}
      />
      {termTag}
    </div>
  );
  const optionalHeard = open.mode === "which" && choice === "raw";
  const heardField = (
    <Input
      label={
        optionalHeard ? t("history.fix.card.heardOptional") : t("history.addToDictionary.heard")
      }
      mono
      value={heard}
      placeholder={optionalHeard ? t("history.addToDictionary.heardHelp") : undefined}
      help={optionalHeard ? undefined : t("history.addToDictionary.heardHelp")}
      error={dict.problem?.heard}
      onChange={(e) => {
        setHeard(e.target.value);
        edited();
      }}
    />
  );

  return (
    <div
      ref={panel}
      role="dialog"
      aria-label={title}
      data-testid="fix-card"
      data-kind={kind}
      data-tauri-drag-region="false"
      style={style}
      // The diff view's question is a little wider (board 05).
      className={cx(CARD_CLASS, pair === undefined ? "w-[400px]" : "w-[420px]")}
      onKeyDown={onKeyDown}>
      <div className="mb-4 flex items-center justify-between gap-3">
        {open.mode === "which" ? (
          <h2 className="text-[14px] font-medium">{t("history.fix.which.title")}</h2>
        ) : (
          <Segmented
            label={t("history.fix.card.kindLabel")}
            size="sm"
            value={kind}
            onChange={switchKind}
            options={[
              { value: "dictionary", label: t("history.fix.card.kindDictionary") },
              { value: "rule", label: t("history.fix.card.kindRule") },
            ]}
          />
        )}
        <IconButton
          icon="x"
          label={t("common.close")}
          onClick={() => {
            onClose(true);
          }}
        />
      </div>

      <div id={fieldsId} className="flex flex-col gap-3">
        {pair !== undefined && (
          <fieldset
            aria-label={t("history.fix.which.title")}
            className="flex flex-col gap-2 border-b border-border pb-3">
            {(["polished", "raw"] as const).map((option) => (
              <label key={option} className="flex cursor-pointer items-start gap-2.5">
                <input
                  type="radio"
                  name={`${fieldsId}-which`}
                  checked={choice === option}
                  aria-label={t(`history.fix.which.${option}`, {
                    add: pair.polished,
                    del: pair.heard,
                  })}
                  onChange={() => {
                    choose(option);
                  }}
                  className="mt-0.5 size-4 accent-[var(--primary)]"
                />
                <span className="min-w-0">
                  <span className={cx("block", choice === option && "font-medium")} data-user-text>
                    {t(
                      option === "polished"
                        ? "history.fix.which.polishedTitle"
                        : "history.fix.which.rawTitle",
                      {
                        add: pair.polished,
                        del: pair.heard,
                      },
                    )}
                  </span>
                  <span className="block text-[12px] text-fg-muted" data-user-text>
                    {t(
                      option === "polished"
                        ? "history.fix.which.polishedHelp"
                        : "history.fix.which.rawHelp",
                      {
                        add: pair.polished,
                        del: pair.heard,
                      },
                    )}
                  </span>
                </span>
              </label>
            ))}
          </fieldset>
        )}

        {kind === "dictionary" ? (
          optionalHeard ? (
            <>
              {termField}
              {heardField}
            </>
          ) : pair !== undefined ? (
            <div className="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-start gap-2">
              {heardField}
              <Icon name="arrowRight" size={16} className="mt-8 text-fg-subtle" />
              {termField}
            </div>
          ) : (
            <>
              {heardField}
              {termField}
            </>
          )
        ) : (
          <RuleFields
            find={find}
            replace={replace}
            name={ruleName}
            ruleKind={ruleKind}
            caseSensitive={caseSensitive}
            polishOnly={open.mode === "fix" && open.polishOnly}
            findError={saveError}
            nameError={ruleName.trim() === "" ? undefined : rule.problem?.name}
            onFind={(v) => {
              setFind(v);
              edited();
            }}
            onReplace={(v) => {
              setReplace(v);
              edited();
            }}
            onName={(v) => {
              setName(v);
              edited();
            }}
            onRuleKind={(v) => {
              setRuleKind(v);
              edited();
            }}
            onCaseSensitive={(v) => {
              setCaseSensitive(v);
              edited();
            }}
          />
        )}

        {kind === "rule" && preview.kind === "ok" && (
          <RulePreview
            before={entry.text}
            after={preview.preview.output}
            fired={preview.preview.rules.some((r) => r.id === NIL_ID)}
            pattern={find}
          />
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

        <p className="text-[11px] leading-[1.5] text-fg-subtle" data-testid="fix-note">
          {kind === "rule"
            ? t("history.fix.card.ruleNote")
            : optionalHeard
              ? t("history.fix.which.rawNote")
              : t("history.fix.card.dictionaryNote")}
        </p>
      </div>

      <div className="mt-4 flex items-center justify-end gap-2">
        <Button
          size="md"
          variant="ghost"
          onClick={() => {
            onClose(true);
          }}>
          {t("common.cancel")}
          <KeyHint keys="Esc" />
        </Button>
        <Button
          size="md"
          variant="primary"
          disabled={!ready}
          data-testid="fix-save"
          onClick={() => {
            void save();
          }}>
          {primary}
          <KeyHint keys="↵" onPrimary />
        </Button>
      </div>
    </div>
  );
}

function RuleFields({
  find,
  replace,
  name,
  ruleKind,
  caseSensitive,
  polishOnly,
  findError,
  nameError,
  onFind,
  onReplace,
  onName,
  onRuleKind,
  onCaseSensitive,
}: {
  find: string;
  replace: string;
  name: string;
  ruleKind: RuleKind;
  caseSensitive: boolean;
  polishOnly: boolean;
  findError?: string | undefined;
  nameError?: string | undefined;
  onFind: (v: string) => void;
  onReplace: (v: string) => void;
  onName: (v: string) => void;
  onRuleKind: (v: RuleKind) => void;
  onCaseSensitive: (v: boolean) => void;
}) {
  const { t } = useI18n();
  return (
    <>
      {polishOnly && (
        <p className="text-[12px] text-fg-muted" data-testid="fix-polish-only">
          {t("history.fix.card.polishOnly")}
        </p>
      )}
      <div className="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-start gap-2">
        <Input
          label={t("history.fix.card.find")}
          mono
          value={find}
          error={findError}
          onChange={(e) => {
            onFind(e.target.value);
          }}
        />
        <Icon name="arrowRight" size={16} className="mt-8 text-fg-subtle" />
        <Input
          label={t("history.fix.card.replace")}
          mono
          value={replace}
          data-autofocus="rule"
          onChange={(e) => {
            onReplace(e.target.value);
          }}
        />
      </div>
      <div className="flex flex-wrap items-center gap-4">
        <Segmented
          label={t("rules.editor.kind")}
          size="sm"
          value={ruleKind}
          onChange={onRuleKind}
          options={[
            { value: "literal", label: t("rules.kind.literal") },
            { value: "regex", label: t("rules.kind.regex") },
          ]}
        />
        <Toggle
          checked={caseSensitive}
          onChange={onCaseSensitive}
          label={t("rules.editor.caseSensitive")}
        />
      </div>
      <Input
        label={t("history.fix.card.name")}
        value={name}
        error={nameError}
        onChange={(e) => {
          onName(e.target.value);
        }}
      />
    </>
  );
}

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
  const { t } = useI18n();
  const segments = fired ? excerptSegments(before, after) : [];
  return (
    <div>
      <div className="mb-1 text-[12px] text-fg-muted">{t("history.fix.card.preview")}</div>
      <div
        className="rounded-10 bg-inset px-3 py-2.5 text-[13px] leading-6"
        data-testid="fix-preview">
        {segments.length === 0 ? (
          <span className="text-fg-muted">{t("history.fix.card.previewNone", { pattern })}</span>
        ) : (
          <span data-user-text>
            {segments.map((seg, i) =>
              seg.kind === "same" ? (
                <span key={i}>{seg.text}</span>
              ) : (
                <span
                  key={i}
                  className={cx(
                    "mx-0.5 rounded-4 px-1",
                    seg.kind === "del"
                      ? "bg-diff-del text-danger line-through"
                      : "bg-diff-add text-ok-text",
                  )}>
                  {seg.text}
                </span>
              ),
            )}
          </span>
        )}
      </div>
    </div>
  );
}

function conflictText(conflict: DictionaryConflict, t: ReturnType<typeof useI18n>["t"]): string {
  switch (conflict.kind) {
    case "taken":
      return t("history.fix.card.taken", { heard: conflict.heard, term: conflict.owner.term });
    case "isTerm":
      return t("history.fix.card.isTerm", { heard: conflict.heard });
    case "same":
      return t("history.fix.card.same");
    case "termTaken":
      return t("history.fix.card.termTaken", { term: conflict.term, owner: conflict.owner.term });
    case "tooMany":
      return t("history.fix.card.tooMany", { max: conflict.max });
    case "full":
      return t("history.fix.card.full", { max: conflict.max });
    case "exists":
      return t("history.fix.card.exists", { term: conflict.entry.term });
  }
}

/** One row: its tone, its words, and the link at its end. */
interface RowView {
  tone: "info" | "warn" | "error" | "busy";
  text: string;
  link?: { label: string; onClick: () => void; expands?: boolean };
}

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
  const { t } = useI18n();
  const show =
    examples.length > 0
      ? {
          label: expanded ? t("common.collapse") : t("history.fix.card.view"),
          onClick: onToggle,
          expands: true,
        }
      : undefined;
  const views: RowView[] = rows.map((row): RowView => {
    switch (row.kind) {
      case "conflict": {
        const c = row.conflict;
        const link =
          c.kind === "exists"
            ? { label: t("history.fix.card.viewEntry"), onClick: onOpenDictionary }
            : c.kind === "taken" || c.kind === "isTerm" || c.kind === "termTaken"
              ? { label: t("history.fix.card.openDictionary"), onClick: onOpenDictionary }
              : undefined;
        return { tone: "error", text: conflictText(c, t), ...(link === undefined ? {} : { link }) };
      }
      case "error":
        return { tone: "error", text: row.message };
      case "merge":
        return {
          tone: "info",
          text: t("history.fix.card.merge", {
            term: row.entry.term,
            heard,
            n: row.entry.heard_as.length,
          }),
        };
      case "computing":
        return { tone: "busy", text: t("history.fix.card.computing") };
      case "impact": {
        const text = rule
          ? row.matched > 0
            ? t("history.fix.card.ruleImpact", { scanned: row.scanned, n: row.matched })
            : t("history.fix.card.ruleImpactNone", { scanned: row.scanned })
          : row.matched > 0
            ? t("history.fix.card.impact", { scanned: row.scanned, n: row.matched, heard })
            : t("history.fix.card.impactNone", { scanned: row.scanned, heard });
        return {
          tone: "info",
          text,
          ...(show === undefined || rows.some((r) => r.kind === "kept") ? {} : { link: show }),
        };
      }
      case "kept":
        return {
          tone: "warn",
          text: t("history.fix.card.kept", { n: row.kept, heard }),
          ...(show === undefined ? {} : { link: show }),
        };
      case "short":
        return {
          tone: "warn",
          text: row.short.latin
            ? t("history.fix.card.shortLatin", { n: row.short.chars })
            : t("history.fix.card.short"),
        };
    }
  });
  const worst = views.some((v) => v.tone === "error")
    ? "error"
    : views.some((v) => v.tone === "warn")
      ? "warn"
      : "info";
  return (
    <div
      role="status"
      aria-live="polite"
      data-testid="fix-status"
      data-tone={worst}
      className={cx(
        "flex flex-col gap-2 rounded-10 px-3 py-2.5 text-[12px] leading-[18px]",
        worst === "error" ? "bg-danger-soft" : worst === "warn" ? "bg-warning-soft" : "bg-inset",
      )}>
      {views.map((v, i) => (
        <StatusLine key={i} view={v} expanded={expanded} />
      ))}
      {expanded && examples.length > 0 && <Examples examples={examples} />}
    </div>
  );
}

function StatusLine({ view, expanded }: { view: RowView; expanded: boolean }) {
  const color =
    view.tone === "error" ? "text-danger" : view.tone === "warn" ? "text-warning" : "text-fg-muted";
  const icon: ReactNode =
    view.tone === "busy" ? (
      <span
        aria-hidden
        className="mt-0.5 inline-block size-3.5 shrink-0 animate-spin rounded-pill border-2 border-primary/30 border-t-primary"
      />
    ) : (
      <Icon
        name={view.tone === "info" ? "info" : "alert"}
        size={14}
        className={cx("mt-0.5 shrink-0", color)}
      />
    );
  return (
    <div className="flex items-start gap-2.5" data-testid="fix-status-row" data-tone={view.tone}>
      {icon}
      <p
        className={cx(
          "min-w-0",
          view.tone === "warn" || view.tone === "error" ? color : "text-fg",
        )}>
        <span data-user-text>{view.text}</span>
        {view.link !== undefined && (
          <button
            type="button"
            className="ml-1 font-medium text-accent-text hover:text-accent-text-hover"
            aria-expanded={view.link.expands === true ? expanded : undefined}
            onClick={view.link.onClick}>
            {view.link.label}
          </button>
        )}
      </p>
    </div>
  );
}

/** Up to three takes the draft would change: when, and the words around the change, the changed
 *  words lit (§16.9 3.3). */
function Examples({ examples }: { examples: readonly BacktestExample[] }) {
  const { locale } = useI18n();
  return (
    <ul className="ml-6 flex flex-col" data-testid="fix-examples">
      {examples.map((e) => {
        const [lead, hit, tail] = snippetParts(e.before, e.after);
        const when = `${formatDateTime(locale, e.at_ms, {
          month: locale === "zh-CN" ? "long" : "short",
          day: "numeric",
        })} ${shortClockLabel(e.at_ms)}`;
        return (
          <li key={e.history_id} className="border-t border-border py-1.5">
            <div className="text-[11px] text-fg-subtle">{when}</div>
            <div className="text-fg" data-user-text>
              {lead}
              <span className="font-medium text-accent-text">{hit}</span>
              {tail}
            </div>
          </li>
        );
      })}
    </ul>
  );
}

export interface KnownCardProps {
  mark: TextMark;
  anchor: () => FloatBox | undefined;
  onClose: (refocus: boolean) => void;
  onOpenDictionary: () => void;
}

/** What a 已加入词典 mark opens (§16.9 3.1): the entry's right spelling and forms, how often it
 *  fired, and the way to edit it in the dictionary. */
export function KnownCard({ mark, anchor, onClose, onOpenDictionary }: KnownCardProps) {
  const { t } = useI18n();
  const panel = useRef<HTMLDivElement>(null);
  const { style } = useFloatingStyle(panel, anchor, { side: "bottom", align: "start" });
  const hits = useHitTotals("corrections");
  const entry = mark.entry;
  useEffect(() => {
    panel.current?.querySelector<HTMLButtonElement>("[data-autofocus]")?.focus();
  }, []);
  if (entry === undefined) return null;
  return (
    <div
      ref={panel}
      role="dialog"
      aria-label={t("history.fix.marks.known")}
      data-testid="fix-known"
      data-tauri-drag-region="false"
      style={style}
      className={cx(CARD_CLASS, "w-[320px]")}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          onClose(true);
          return;
        }
        trapTab(e);
      }}>
      <div className="mb-3 flex items-center justify-between">
        <h2 className="flex items-center gap-2 text-[14px] font-medium">
          <span aria-hidden className="h-[1.5px] w-4 bg-ok-text" />
          {t("history.fix.marks.known")}
        </h2>
        <IconButton
          icon="x"
          label={t("common.close")}
          onClick={() => {
            onClose(true);
          }}
        />
      </div>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-[13px]">
        <dt className="text-fg-muted">{t("history.fix.known.term")}</dt>
        <dd className="mono" data-user-text>
          {entry.term}
        </dd>
        <dt className="text-fg-muted">{t("history.fix.known.heard")}</dt>
        <dd className="mono" data-user-text>
          {entry.heard_as.join(HEARD_AS_JOINER)}
        </dd>
        <dt className="text-fg-muted">{t("history.fix.known.hits")}</dt>
        <dd className="mono">{t("history.fix.known.hitsValue", { n: hits.get(entry.id) ?? 0 })}</dd>
      </dl>
      <div className="mt-4 flex justify-end">
        <Button size="md" icon="book" data-autofocus="" onClick={onOpenDictionary}>
          {t("history.fix.known.edit")}
        </Button>
      </div>
    </div>
  );
}
