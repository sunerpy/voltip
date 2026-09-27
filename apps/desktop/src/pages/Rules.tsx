import {
  type ImportMode,
  MAX_RULES,
  NIL_ID,
  type PreviewDraft,
  type ReplacementRule,
  type RuleDraft,
  type VocabularyPreview,
} from "@voltip/shared";
import {
  Badge,
  Button,
  Card,
  Dialog,
  EmptyState,
  IconButton,
  Input,
  Lamp,
  Panel,
  Segmented,
  Table,
  type TableColumn,
  Textarea,
  Toggle,
  cx,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { type KeyboardEvent, useCallback, useEffect, useMemo, useState } from "react";
import { usePageShortcuts, withCommand } from "../app/page-shortcuts";
import { useRouter } from "../app/router";
import { copyWithToast, useShell } from "../app/shell-context";
import { useVocabularyPreview } from "../features/vocabulary/usePreview";
import { errorText, hitTotals, moved, ruleDraftProblem } from "../features/vocabulary/vocabulary";

type Kind = "all" | "literal" | "regex";

/** The rule being edited; `id` is absent for a new rule (the editor sits above the table). */
interface Draft extends RuleDraft {
  id: string | undefined;
}

function emptyDraft(): Draft {
  return {
    id: undefined,
    name: "",
    kind: "literal",
    pattern: "",
    replacement: "",
    case_sensitive: true,
    enabled: true,
  };
}

function draftFrom(rule: ReplacementRule): Draft {
  return {
    id: rule.id,
    name: rule.name,
    kind: rule.kind,
    pattern: rule.pattern,
    replacement: rule.replacement,
    case_sensitive: rule.case_sensitive,
    enabled: rule.enabled,
  };
}

function ruleOf({ id: _id, ...rule }: Draft): RuleDraft {
  return rule;
}

type DryRun =
  | { kind: "result"; input: string; preview: VocabularyPreview; withDraft: boolean }
  | { kind: "error"; message: string };

/** Replacement rules (docs/dictation.md §16): the core's `state.rules` in execution order. Add /
 *  edit / enable / reorder / delete go through the `rules_*` commands; the editor has the core
 *  check the draft (`vocabulary_preview` with it), so a regex is judged by the Rust dialect the
 *  pipeline runs; TOML import / export are text dialogs over `rules_import` / `rules_export`; the
 *  dry run is the core's preview of the dictionary plus every enabled rule. */
export interface RulesProps {
  /** Open the editor on a new rule once (`/rules?new=1`, the palette's 新建规则). */
  compose?: boolean;
}

export function Rules({ compose = false }: RulesProps) {
  const shell = useShell();
  const { navigate } = useRouter();
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const rules = state.rules;
  const [kind, setKind] = useState<Kind>("all");
  const [query, setQuery] = useState("");
  const [draft, setDraft] = useState<Draft | undefined>(undefined);
  const [saveError, setSaveError] = useState<string | undefined>(undefined);
  const [input, setInput] = useState("");
  const [withDraft, setWithDraft] = useState(true);
  const [result, setResult] = useState<DryRun | undefined>(undefined);
  const [importing, setImporting] = useState(false);
  const [exported, setExported] = useState<string | undefined>(undefined);

  const hits = useMemo(() => hitTotals(state.history, "rules"), [state.history]);
  const enabled = rules.filter((r) => r.enabled).length;
  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return rules.filter((r) => {
      if (kind !== "all" && r.kind !== kind) return false;
      return (
        q.length === 0 || r.name.toLowerCase().includes(q) || r.pattern.toLowerCase().includes(q)
      );
    });
  }, [rules, kind, query]);
  const problem = draft
    ? ruleDraftProblem(
        draft.name,
        draft.pattern,
        rules.filter((r) => r.id !== draft.id),
        t,
      )
    : undefined;
  // The core checks the draft itself (a regex is compiled by the pipeline's own engine).
  const previewDraft = useMemo<PreviewDraft | undefined>(
    () => (draft && !problem ? { id: draft.id ?? null, rule: ruleOf(draft) } : undefined),
    [draft, problem],
  );
  const check = useVocabularyPreview("", previewDraft, previewDraft !== undefined);

  const startNew = useCallback(() => {
    setDraft(emptyDraft());
    setSaveError(undefined);
  }, []);
  // `/rules?new=1` opens the editor on a new rule: adjusted during render when the flag appears
  // (React's pattern for state that follows a prop), then the effect drops the flag from the URL
  // so the palette can ask again and history holds no one-shot flag.
  const [composing, setComposing] = useState(false);
  if (composing !== compose) {
    setComposing(compose);
    if (compose) startNew();
  }
  useEffect(() => {
    if (compose) navigate({ name: "rules" }, { replace: true });
  }, [compose, navigate]);
  // Ctrl N: a new rule (not while a modal is over the page).
  usePageShortcuts((e) => {
    if (!withCommand(e) || e.key.toLowerCase() !== "n") return false;
    startNew();
    return true;
  });

  const run = (action: Promise<void>) => {
    action.catch((e: unknown) => {
      shell.toast({
        message: t("app.error", { message: errorText(e) }),
        duration: 5000,
        tone: "danger",
      });
    });
  };
  const save = async () => {
    if (!draft || problem) return;
    try {
      if (draft.id === undefined) await backend.invoke("rules_add", { rule: ruleOf(draft) });
      else await backend.invoke("rules_update", { id: draft.id, rule: ruleOf(draft) });
      setDraft(undefined);
      setSaveError(undefined);
    } catch (e) {
      setSaveError(errorText(e));
    }
  };
  const remove = (rule: ReplacementRule) => {
    shell.confirm({
      title: t("rules.confirm.deleteTitle", { name: rule.name }),
      body: t("rules.confirm.deleteBody"),
      confirmLabel: t("common.delete"),
      tone: "danger",
      onConfirm: () => {
        if (draft?.id === rule.id) setDraft(undefined);
        run(backend.invoke("rules_remove", { id: rule.id }));
      },
    });
  };
  const move = (rule: ReplacementRule, delta: -1 | 1) => {
    const ids = rules.map((r) => r.id);
    run(backend.invoke("rules_reorder", { ids: moved(ids, ids.indexOf(rule.id), delta) }));
  };
  const dryRun = () => {
    const include = withDraft && previewDraft !== undefined;
    backend.vocabularyPreview(input, include ? previewDraft : undefined).then(
      (preview) => {
        setResult({ kind: "result", input, preview, withDraft: include });
      },
      (e: unknown) => {
        setResult({ kind: "error", message: errorText(e) });
      },
    );
  };
  const openExport = () => {
    backend.rulesExport().then(setExported, (e: unknown) => {
      shell.toast({
        message: t("rules.exportDialog.failed", { message: errorText(e) }),
        duration: 5000,
        tone: "danger",
      });
    });
  };
  const nameOf = (id: string) =>
    id === NIL_ID || (result?.kind === "result" && result.withDraft && id === draft?.id)
      ? t("rules.dryRun.draftName", { name: draft?.name ?? "" })
      : (rules.find((r) => r.id === id)?.name ?? id);

  const columns: TableColumn<ReplacementRule>[] = [
    {
      id: "order",
      header: t("rules.column.order"),
      width: 32,
      align: "right",
      cell: (r) => ({ type: "mono", text: rules.indexOf(r) + 1, muted: true }),
    },
    {
      id: "name",
      header: t("rules.column.name"),
      width: 120,
      cell: (r) => (
        <span data-user-text className={cx("mono", r.enabled ? "text-fg" : "text-fg-subtle")}>
          {r.name}
        </span>
      ),
    },
    {
      id: "kind",
      header: t("rules.column.kind"),
      width: 64,
      mono: false,
      cell: (r) => ({
        type: "badge",
        text: t(`rules.kind.${r.kind}`),
        tone: r.kind === "regex" ? "accent" : "neutral",
      }),
    },
    {
      id: "from",
      header: t("rules.column.from"),
      cell: (r) => (
        <span className="mono flex items-center gap-1.5">
          <span
            data-user-text
            className={cx("min-w-0 truncate", r.enabled ? "text-fg" : "text-fg-subtle")}>
            {r.pattern}
          </span>
          {!r.case_sensitive && (
            <span
              className="rounded-4 bg-inset px-1 text-[10px] text-fg-subtle"
              title={t("rules.caseInsensitive")}>
              Aa
            </span>
          )}
        </span>
      ),
    },
    {
      id: "to",
      header: t("rules.column.to"),
      width: 120,
      cell: (r) =>
        r.replacement.length === 0 ? (
          { type: "mono", text: t("rules.column.emptyTo"), muted: true }
        ) : (
          <span data-user-text className="mono">
            {r.replacement}
          </span>
        ),
    },
    {
      id: "hits",
      header: t("rules.column.hits"),
      width: 48,
      align: "right",
      cell: (r) => {
        const n = hits.get(r.id) ?? 0;
        return { type: "mono", text: n > 0 ? String(n) : "—", muted: n === 0 };
      },
    },
    {
      id: "enabled",
      header: t("rules.column.enabled"),
      width: 48,
      align: "center",
      mono: false,
      cell: (r) => ({
        type: "toggle",
        checked: r.enabled,
        label: t("rules.row.enable", { name: r.name }),
        onChange: (next) => {
          run(
            backend.invoke("rules_update", {
              id: r.id,
              rule: { ...ruleOf(draftFrom(r)), enabled: next },
            }),
          );
        },
      }),
    },
    {
      id: "actions",
      header: "",
      width: 120,
      align: "right",
      mono: false,
      cell: (r) => (
        <span className="inline-flex gap-0.5">
          <IconButton
            icon="chevronUp"
            label={t("rules.row.up", { name: r.name })}
            disabled={rules[0]?.id === r.id}
            onClick={() => {
              move(r, -1);
            }}
          />
          <IconButton
            icon="chevronDown"
            label={t("rules.row.down", { name: r.name })}
            disabled={rules.at(-1)?.id === r.id}
            onClick={() => {
              move(r, 1);
            }}
          />
          <IconButton
            icon="edit"
            label={t("rules.row.edit", { name: r.name })}
            onClick={() => {
              setDraft(draftFrom(r));
              setSaveError(undefined);
            }}
          />
          <IconButton
            icon="trash"
            label={t("rules.row.delete", { name: r.name })}
            tone="danger"
            onClick={() => {
              remove(r);
            }}
          />
        </span>
      ),
    },
  ];

  const editorKeys = (e: KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      void save();
    } else if (e.key === "Escape") {
      e.preventDefault();
      setDraft(undefined);
    }
  };
  const status =
    saveError ??
    problem?.name ??
    problem?.pattern ??
    (check.kind === "error" ? check.message : undefined);
  const editor = draft && (
    <Card
      padding="sm"
      className="my-2 grid grid-cols-2 gap-3"
      data-testid="rule-editor"
      onKeyDown={editorKeys}>
      <div className="eyebrow col-span-2">
        {t("rules.editor.title", {
          name: draft.id === undefined ? t("rules.editor.newRule") : draft.name,
        })}
      </div>
      <Input
        label={t("rules.editor.name")}
        mono
        size="sm"
        value={draft.name}
        autoFocus
        error={problem?.name}
        onChange={(e) => {
          setDraft({ ...draft, name: e.target.value });
          setSaveError(undefined);
        }}
      />
      <div>
        <div className="mb-1 text-[12px] text-fg-muted">{t("rules.editor.kind")}</div>
        <Segmented
          label={t("rules.editor.kind")}
          size="sm"
          value={draft.kind}
          onChange={(k) => {
            setDraft({ ...draft, kind: k });
            setSaveError(undefined);
          }}
          options={[
            { value: "literal", label: t("rules.kind.literal") },
            { value: "regex", label: t("rules.kind.regex") },
          ]}
        />
      </div>
      <Input
        label={t("rules.editor.from")}
        mono
        size="sm"
        value={draft.pattern}
        error={problem?.pattern ?? (check.kind === "error" ? check.message : undefined)}
        onChange={(e) => {
          setDraft({ ...draft, pattern: e.target.value });
          setSaveError(undefined);
        }}
      />
      <Input
        label={t("rules.editor.to")}
        mono
        size="sm"
        value={draft.replacement}
        onChange={(e) => {
          setDraft({ ...draft, replacement: e.target.value });
          setSaveError(undefined);
        }}
        help={t("rules.editor.toHelp")}
      />
      <Toggle
        checked={draft.case_sensitive}
        onChange={(v) => {
          setDraft({ ...draft, case_sensitive: v });
        }}
        label={t("rules.editor.caseSensitive")}
      />
      <div className="col-span-2 flex items-center gap-2">
        <span
          className={`mono flex-1 text-[11px] ${status ? "text-danger" : "text-fg-muted"}`}
          role="status"
          data-testid="draft-status">
          {status ?? (check.kind === "ok" ? t("rules.editor.ok") : t("rules.editor.checking"))}
        </span>
        <Button
          size="sm"
          variant="primary"
          onClick={() => {
            void save();
          }}
          disabled={problem !== undefined || check.kind === "error"}
          keys="Ctrl S">
          {t("common.save")}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            setDraft(undefined);
          }}
          keys="Esc">
          {t("common.cancel")}
        </Button>
      </div>
    </Card>
  );

  return (
    <div className="mx-auto flex w-full max-w-[1440px] flex-col gap-3 p-6" data-testid="page-rules">
      <div className="flex flex-wrap items-center gap-3">
        <h2 className="text-[18px] font-semibold text-fg">{t("rules.title")}</h2>
        <Badge mono>{t("rules.countBadge", { n: rules.length })}</Badge>
        <Badge tone="ok">{t("rules.enabledBadge", { n: enabled })}</Badge>
        <div className="ml-auto flex gap-2">
          <Button
            size="sm"
            icon="upload"
            onClick={() => {
              setImporting(true);
            }}>
            {t("rules.importToml")}
          </Button>
          <Button size="sm" icon="download" onClick={openExport}>
            {t("rules.exportToml")}
          </Button>
          <Button
            size="sm"
            variant="primary"
            icon="plus"
            keys="Ctrl N"
            disabled={rules.length >= MAX_RULES}
            onClick={startNew}>
            {t("rules.newRule")}
          </Button>
        </div>
      </div>
      <p className="text-[12px] text-fg-muted">{t("rules.intro")}</p>

      {/* rule table 3/5, dry run 2/5 (never under 320 px); below `lg` they stack. */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,3fr)_minmax(320px,2fr)]">
        <Panel
          eyebrow={t("rules.eyebrow")}
          className="min-w-0"
          right={
            <div className="flex flex-wrap items-center justify-end gap-2">
              <Segmented
                label={t("rules.kindLabel")}
                size="sm"
                variant="ink"
                value={kind}
                onChange={setKind}
                options={[
                  { value: "all", label: t("rules.kind.all") },
                  { value: "literal", label: t("rules.kind.literal") },
                  { value: "regex", label: t("rules.kind.regex") },
                ]}
              />
              <Input
                icon="search"
                size="sm"
                placeholder={t("rules.searchPlaceholder")}
                value={query}
                onChange={(e) => {
                  setQuery(e.target.value);
                }}
                className="w-44"
                aria-label={t("rules.searchLabel")}
              />
            </div>
          }
          padding="sm">
          {draft?.id === undefined && editor}
          {rules.length === 0 ? (
            <EmptyState
              title={t("rules.empty.title")}
              actions={
                <Button size="sm" variant="primary" onClick={startNew}>
                  {t("rules.empty.newRule")}
                </Button>
              }>
              {t("rules.empty.body")}
            </EmptyState>
          ) : visible.length === 0 ? (
            <EmptyState compact title={t("rules.empty.noMatch")} />
          ) : (
            <Table
              label={t("rules.eyebrow")}
              columns={columns}
              rows={visible}
              rowKey={(r) => r.id}
              dense
              selectedKey={draft?.id}
              expandRow={(r) => (draft?.id === r.id ? editor : null)}
              rowClassName={(r) => (r.enabled ? undefined : "opacity-60")}
            />
          )}
          <div className="mono mt-3 text-[11px] text-fg-subtle">
            {t("rules.footer", { n: rules.length, limit: MAX_RULES })}
          </div>
        </Panel>

        <Panel
          eyebrow={t("rules.dryRun.title")}
          radius={14}
          className="min-w-0"
          bodyClassName="flex flex-col gap-3">
          <p className="text-[12px] text-fg-muted">{t("rules.dryRun.intro")}</p>
          <Textarea
            aria-label={t("rules.dryRun.inputLabel")}
            mono
            rows={3}
            value={input}
            placeholder={t("rules.dryRun.placeholder")}
            onChange={(e) => {
              setInput(e.target.value);
            }}
            onKeyDown={(e) => {
              if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
                e.preventDefault();
                dryRun();
              }
            }}
          />
          <div className="flex flex-wrap items-center gap-2">
            <Button size="sm" variant="primary" onClick={dryRun} keys="Ctrl ↵">
              {t("rules.dryRun.run")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              disabled={state.history[0] === undefined}
              onClick={() => {
                setInput(state.history[0]?.raw_text ?? "");
              }}>
              {t("rules.dryRun.lastDictation")}
            </Button>
            {draft && (
              <Toggle
                checked={withDraft}
                onChange={setWithDraft}
                label={t("rules.dryRun.withDraft")}
                className="ml-auto"
              />
            )}
          </div>
          <DryRunResult
            result={result}
            nameOf={nameOf}
            onCopy={(text) => {
              void copyWithToast(shell, text, t("rules.dryRun.copiedResult"));
            }}
          />
        </Panel>
      </div>
      {importing && (
        <ImportDialog
          onClose={() => {
            setImporting(false);
          }}
        />
      )}
      {exported !== undefined && (
        <Dialog
          open
          title={t("rules.exportDialog.title")}
          width={600}
          onClose={() => {
            setExported(undefined);
          }}
          actions={
            <>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  setExported(undefined);
                }}
                data-autofocus>
                {t("common.close")}
              </Button>
              <Button
                size="sm"
                variant="primary"
                icon="copy"
                onClick={() => {
                  void copyWithToast(shell, exported, t("rules.exportDialog.copied"));
                }}>
                {t("rules.exportDialog.copy")}
              </Button>
            </>
          }>
          <p className="mb-2">{t("rules.exportDialog.body")}</p>
          <Textarea
            aria-label={t("rules.exportDialog.label")}
            mono
            rows={14}
            readOnly
            value={exported}
            data-testid="rules-export-text"
          />
        </Dialog>
      )}
    </div>
  );
}

function DryRunResult({
  result,
  nameOf,
  onCopy,
}: {
  result: DryRun | undefined;
  nameOf: (id: string) => string;
  onCopy: (text: string) => void;
}) {
  const { t } = useI18n();
  if (result === undefined)
    return (
      <div className="rounded-6 bg-inset p-3 text-[12px] text-fg-subtle">
        {t("rules.dryRun.prompt")}
      </div>
    );
  if (result.kind === "error")
    return (
      <div className="text-[12px] text-danger" role="status" data-testid="dry-run-error">
        {t("rules.dryRun.failed", { message: result.message })}
      </div>
    );
  const { corrected, output, rules, error } = result.preview;
  const changes = rules.reduce((sum, hit) => sum + hit.count, 0);
  return (
    <>
      <div className="rounded-6 bg-inset p-3" data-testid="dry-run-result">
        <div className="eyebrow">{t("rules.dryRun.corrected")}</div>
        <div className="mono text-[13px] text-fg" data-user-text data-testid="dry-run-corrected">
          {corrected}
        </div>
        <div className="eyebrow mt-2">{t("rules.dryRun.after")}</div>
        <div className="mono text-[13px] text-fg" data-user-text data-testid="dry-run-after">
          {output}
        </div>
        <div className="mono mt-2 text-[11px] text-fg-subtle">
          {rules.length === 0 && output === result.input
            ? t("rules.dryRun.noChange")
            : t("rules.dryRun.summary", { hits: rules.length, changes })}
        </div>
        <IconButton
          icon="copy"
          label={t("rules.dryRun.copyResult")}
          className="mt-1"
          onClick={() => {
            onCopy(output);
          }}
        />
      </div>
      {rules.length > 0 && (
        <ul className="flex flex-col divide-y divide-border" aria-label={t("rules.dryRun.hitList")}>
          {rules.map((hit, i) => (
            <li key={hit.id} className="flex items-center gap-2 py-1.5 text-[12px]">
              <Lamp tone="ok" size={6} />
              <span className="mono text-fg-subtle">{i + 1}</span>
              <span className="mono flex-1 truncate text-fg" data-user-text>
                {nameOf(hit.id)}
              </span>
              <span className="mono text-fg-muted">×{hit.count}</span>
            </li>
          ))}
        </ul>
      )}
      {error !== undefined && (
        <div className="border-l-2 border-warning pl-2 text-[11px] text-fg-muted">
          {t("rules.dryRun.fallback", { reason: error })}
        </div>
      )}
    </>
  );
}

/** Paste a TOML text, pick merge or replace; the core parses and validates the whole file
 *  synchronously, so a refusal comes back here with its position and nothing is imported. */
function ImportDialog({ onClose }: { onClose: () => void }) {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const [text, setText] = useState("");
  const [mode, setMode] = useState<ImportMode>("merge");
  const [error, setError] = useState<string | undefined>(undefined);
  const modeLabel =
    mode === "merge" ? t("rules.importDialog.merge") : t("rules.importDialog.replace");
  const submit = async () => {
    try {
      await backend.invoke("rules_import", { toml: text, mode });
      shell.toast({ message: t("rules.importDialog.done", { mode: modeLabel }), duration: 3000 });
      onClose();
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <Dialog
      open
      title={t("rules.importDialog.title")}
      width={600}
      onClose={onClose}
      actions={
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant="primary"
            disabled={text.trim().length === 0}
            onClick={() => {
              void submit();
            }}>
            {t("rules.importDialog.import")}
          </Button>
        </>
      }>
      <div className="flex flex-col gap-3">
        <p>{t("rules.importDialog.body")}</p>
        <Textarea
          aria-label={t("rules.importDialog.label")}
          mono
          rows={12}
          value={text}
          data-autofocus
          onChange={(e) => {
            setText(e.target.value);
            setError(undefined);
          }}
        />
        <div className="flex flex-wrap items-center gap-3">
          <Segmented
            label={t("rules.importDialog.mode")}
            size="sm"
            value={mode}
            onChange={setMode}
            options={[
              { value: "merge", label: t("rules.importDialog.merge") },
              { value: "replace", label: t("rules.importDialog.replace") },
            ]}
          />
          <span className="text-[12px] text-fg-subtle">
            {mode === "merge"
              ? t("rules.importDialog.mergeHelp")
              : t("rules.importDialog.replaceHelp")}
          </span>
        </div>
        {error !== undefined && (
          <pre
            className="mono max-h-40 overflow-auto whitespace-pre-wrap text-[12px] text-danger"
            role="alert"
            data-testid="rules-import-error">
            {error}
          </pre>
        )}
      </div>
    </Dialog>
  );
}
