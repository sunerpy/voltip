import {
  MAX_RULES,
  type PreviewDraft,
  type ReplacementRule,
  type RuleDraft,
  errorText,
  movedBy,
  ruleDraftProblem,
} from "@voltip/shared";
import {
  Badge,
  Button,
  Card,
  Dialog,
  EmptyState,
  IconButton,
  Input,
  RulesExportDialog,
  RulesImportDialog,
  Segmented,
  Textarea,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
  useVocabularyPreview,
} from "@voltip/ui";
import { useMemo, useState } from "react";
import { Lede, PAGE, TOUCH, TOUCH_ICON, TOUCH_TEXTAREA, TOUCH_TOGGLE } from "../app/phone-ui";
import { useMobileShell } from "../app/shell";

const NEW_RULE: RuleDraft = {
  name: "",
  kind: "literal",
  pattern: "",
  replacement: "",
  case_sensitive: true,
  enabled: true,
};

function draftOf(rule: ReplacementRule): RuleDraft {
  return {
    name: rule.name,
    kind: rule.kind,
    pattern: rule.pattern,
    replacement: rule.replacement,
    case_sensitive: rule.case_sensitive,
    enabled: rule.enabled,
  };
}

/** Creates or edits one rule. The core checks the draft itself (`vocabulary_preview` with it), so
 *  a regex is judged by the Rust dialect the pipeline runs; 试一试 shows a text after the
 *  dictionary and every enabled rule, this draft included. An existing rule also moves in the
 *  order or is deleted after a confirm. */
function RuleDialog({ rule, onClose }: { rule?: ReplacementRule; onClose: () => void }) {
  const { backend } = useBackend();
  const shell = useMobileShell();
  const { t } = useI18n();
  const rules = useUiState().rules;
  const [draft, setDraft] = useState<RuleDraft>(() => (rule ? draftOf(rule) : NEW_RULE));
  const [sample, setSample] = useState("");
  const [saveError, setSaveError] = useState<string | undefined>(undefined);
  const problem = ruleDraftProblem(
    draft.name,
    draft.pattern,
    rules.filter((r) => r.id !== rule?.id),
    t,
  );
  const previewDraft = useMemo<PreviewDraft | undefined>(
    () => (problem === undefined ? { id: rule?.id ?? null, rule: draft } : undefined),
    [draft, problem, rule?.id],
  );
  const check = useVocabularyPreview(sample, previewDraft, previewDraft !== undefined);
  const index = rule === undefined ? -1 : rules.findIndex((r) => r.id === rule.id);
  const update = (patch: Partial<RuleDraft>) => {
    setDraft((d) => ({ ...d, ...patch }));
    setSaveError(undefined);
  };
  const fail = (e: unknown) => {
    shell.toast(t("mobile.toast.error", { message: errorText(e) }), "danger");
  };
  const save = async () => {
    if (problem !== undefined) return;
    try {
      if (rule === undefined) await backend.invoke("rules_add", { rule: draft });
      else await backend.invoke("rules_update", { id: rule.id, rule: draft });
      onClose();
    } catch (e) {
      setSaveError(errorText(e));
    }
  };
  const move = (delta: -1 | 1) => {
    const ids = rules.map((r) => r.id);
    backend.invoke("rules_reorder", { ids: movedBy(ids, index, delta) }).catch(fail);
  };
  const remove = () => {
    if (rule === undefined) return;
    shell.confirm({
      title: t("rules.confirm.deleteTitle", { name: rule.name }),
      body: t("rules.confirm.deleteBody"),
      confirmLabel: t("common.delete"),
      onConfirm: () => {
        backend.invoke("rules_remove", { id: rule.id }).then(onClose, fail);
      },
    });
  };
  const status =
    saveError ??
    (draft.pattern.length > 0 ? problem?.pattern : undefined) ??
    (check.kind === "error" ? check.message : undefined);

  // No key hint under the buttons: a phone has no Esc; its back closes the dialog.
  return (
    <Dialog
      open
      title={rule === undefined ? t("rules.empty.newRule") : t("mobile.rules.editTitle")}
      width={420}
      hint=""
      onClose={onClose}
      actions={
        <>
          {rule !== undefined && (
            <Button variant="text-danger" className={`${TOUCH} mr-auto`} onClick={remove}>
              {t("common.delete")}
            </Button>
          )}
          <Button variant="ghost" className={TOUCH} onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            variant="primary"
            className={TOUCH}
            disabled={problem !== undefined || check.kind === "error"}
            onClick={() => {
              void save();
            }}>
            {t("common.save")}
          </Button>
        </>
      }>
      <div
        className="-mx-1 flex max-h-[60vh] flex-col gap-3 overflow-y-auto overscroll-none px-1"
        data-testid="rule-editor">
        <Input
          label={t("rules.editor.name")}
          size="lg"
          value={draft.name}
          data-autofocus
          error={draft.name.length > 0 ? problem?.name : undefined}
          onChange={(e) => {
            update({ name: e.target.value });
          }}
        />
        <div className="flex flex-col gap-1">
          <span className="text-[12px] text-fg-muted">{t("rules.editor.kind")}</span>
          <Segmented
            label={t("rules.editor.kind")}
            value={draft.kind}
            onChange={(kind) => {
              update({ kind });
            }}
            options={[
              { value: "literal", label: t("rules.kind.literal") },
              { value: "regex", label: t("rules.kind.regex") },
            ]}
            className="h-11 w-full [&>button]:flex-1"
          />
        </div>
        <Input
          label={t("rules.editor.from")}
          mono
          size="lg"
          value={draft.pattern}
          onChange={(e) => {
            update({ pattern: e.target.value });
          }}
        />
        <Input
          label={t("rules.editor.to")}
          mono
          size="lg"
          value={draft.replacement}
          help={t("rules.editor.toHelp")}
          onChange={(e) => {
            update({ replacement: e.target.value });
          }}
        />
        {/* The label takes the taps beside the switch: the row is the target. */}
        <Toggle
          checked={draft.case_sensitive}
          label={t("rules.editor.caseSensitive")}
          className="min-h-11 self-start"
          onChange={(case_sensitive) => {
            update({ case_sensitive });
          }}
        />
        <span
          className={`mono text-[11px] ${status === undefined ? "text-fg-muted" : "text-danger"}`}
          role="status"
          data-testid="rule-status">
          {status ??
            (problem !== undefined
              ? ""
              : check.kind === "ok"
                ? t("rules.editor.ok")
                : t("rules.editor.checking"))}
        </span>
        <Textarea
          label={t("mobile.rules.test")}
          className={TOUCH_TEXTAREA}
          rows={2}
          value={sample}
          placeholder={t("mobile.rules.testPlaceholder")}
          onChange={(e) => {
            setSample(e.target.value);
          }}
        />
        {sample.length > 0 && check.kind === "ok" && (
          <div className="rounded-6 bg-inset p-3" data-testid="rule-test-result">
            <div className="eyebrow">{t("rules.dryRun.after")}</div>
            <div className="mono text-[13px] text-fg" data-user-text>
              {check.preview.output}
            </div>
          </div>
        )}
        {rule !== undefined && rules.length > 1 && (
          <div className="flex items-center gap-1">
            <IconButton
              icon="chevronUp"
              size={28}
              bordered
              label={t("rules.row.up", { name: rule.name })}
              className={TOUCH_ICON}
              disabled={index <= 0}
              onClick={() => {
                move(-1);
              }}
            />
            <IconButton
              icon="chevronDown"
              size={28}
              bordered
              label={t("rules.row.down", { name: rule.name })}
              className={TOUCH_ICON}
              disabled={index === rules.length - 1}
              onClick={() => {
                move(1);
              }}
            />
            <span className="ml-2 text-[12px] text-fg-subtle">
              {t("mobile.dictionary.order", { n: index + 1, total: rules.length })}
            </span>
          </div>
        )}
      </div>
    </Dialog>
  );
}

/** 替换规则 on the phone (docs/dictation.md §16; user decision 2026-10-01): the core's
 *  `state.rules` in execution order; each rule switches on or off here and opens in the editor;
 *  the rules come in as pasted TOML (`rules_import`) and go out copied or through the share sheet
 *  (`rules_export`). */
export function Rules() {
  const { backend } = useBackend();
  const shell = useMobileShell();
  const { t } = useI18n();
  const rules = useUiState().rules;
  // `{}` is a new rule, `{ rule }` an existing one.
  const [editing, setEditing] = useState<{ rule?: ReplacementRule } | undefined>(undefined);
  const [importing, setImporting] = useState(false);
  const [exported, setExported] = useState<string | undefined>(undefined);
  const fail = (e: unknown) => {
    shell.toast(t("mobile.toast.error", { message: errorText(e) }), "danger");
  };
  const toggle = (rule: ReplacementRule, enabled: boolean) => {
    backend
      .invoke("rules_update", { id: rule.id, rule: { ...draftOf(rule), enabled } })
      .catch(fail);
  };
  const openExport = () => {
    backend.rulesExport().then(setExported, (e: unknown) => {
      shell.toast(t("rules.exportDialog.failed", { message: errorText(e) }), "danger");
    });
  };
  const copy = (text: string) => {
    void backend.pasteText(text).then(
      (outcome) => {
        if (outcome.kind === "failed") shell.toast(t("mobile.recent.copyFailed"), "danger");
        else shell.toast(t("rules.exportDialog.copied"));
      },
      () => {
        shell.toast(t("mobile.recent.copyFailed"), "danger");
      },
    );
  };

  return (
    <div className={PAGE} data-testid="phone-rules">
      <Lede>{t("mobile.rules.intro")}</Lede>
      <div className="flex gap-2">
        <Button
          variant="primary"
          icon="plus"
          className={`${TOUCH} min-w-0 flex-1`}
          disabled={rules.length >= MAX_RULES}
          onClick={() => {
            setEditing({});
          }}>
          {t("rules.empty.newRule")}
        </Button>
        <Button
          className={TOUCH}
          onClick={() => {
            setImporting(true);
          }}>
          {t("rules.importToml")}
        </Button>
        <Button className={TOUCH} onClick={openExport}>
          {t("rules.exportToml")}
        </Button>
      </div>
      {rules.length === 0 ? (
        <Card padding="none">
          <EmptyState compact title={t("rules.empty.title")}>
            {t("mobile.rules.emptyBody")}
          </EmptyState>
        </Card>
      ) : (
        <Card padding="none" className="overflow-hidden">
          <ul aria-label={t("mobile.title.rules")} className="flex flex-col divide-y divide-border">
            {rules.map((rule) => (
              <li key={rule.id} className="flex items-center gap-3 pr-4">
                {/* The rule opens in the editor; the switch beside it is a control of its own. */}
                <button
                  type="button"
                  className="flex min-h-14 min-w-0 flex-1 flex-col justify-center gap-0.5 py-3 pl-4 text-left transition-colors hover:bg-inset active:bg-inset"
                  aria-label={t("rules.row.edit", { name: rule.name })}
                  onClick={() => {
                    setEditing({ rule });
                  }}>
                  <span className="flex min-w-0 items-center gap-2">
                    <span
                      className={`truncate text-[14px] font-medium ${rule.enabled ? "text-fg" : "text-fg-subtle"}`}
                      data-user-text>
                      {rule.name}
                    </span>
                    <Badge tone={rule.kind === "regex" ? "accent" : "neutral"} className="shrink-0">
                      {t(`rules.kind.${rule.kind}`)}
                    </Badge>
                  </span>
                  <span className="mono truncate text-[12px] text-fg-muted" data-user-text>
                    {rule.pattern} →{" "}
                    {rule.replacement.length === 0 ? t("rules.column.emptyTo") : rule.replacement}
                  </span>
                </button>
                <Toggle
                  checked={rule.enabled}
                  ariaLabel={t("rules.row.enable", { name: rule.name })}
                  className={TOUCH_TOGGLE}
                  onChange={(enabled) => {
                    toggle(rule, enabled);
                  }}
                />
              </li>
            ))}
          </ul>
        </Card>
      )}
      <p className="mono px-1 text-[11px] text-fg-subtle">
        {t("rules.footer", { n: rules.length, limit: MAX_RULES })}
      </p>
      {editing !== undefined && (
        <RuleDialog
          key={editing.rule?.id ?? "new"}
          rule={editing.rule}
          onClose={() => {
            setEditing(undefined);
          }}
        />
      )}
      {importing && (
        <RulesImportDialog
          onClose={() => {
            setImporting(false);
          }}
        />
      )}
      {exported !== undefined && (
        <RulesExportDialog
          text={exported}
          onClose={() => {
            setExported(undefined);
          }}
          actions={
            <>
              <Button
                icon="share"
                className={TOUCH}
                onClick={() => {
                  backend.invoke("phone_share_text", { text: exported }).catch(fail);
                }}>
                {t("mobile.rules.share")}
              </Button>
              <Button
                variant="primary"
                icon="copy"
                className={TOUCH}
                onClick={() => {
                  copy(exported);
                }}>
                {t("rules.exportDialog.copy")}
              </Button>
            </>
          }
        />
      )}
    </div>
  );
}
