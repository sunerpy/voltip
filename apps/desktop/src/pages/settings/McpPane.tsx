import {
  MCP_HISTORY_RANGES,
  MCP_READS,
  MCP_SETUP_CLIENTS,
  MCP_SETUP_LABELS,
  MCP_WRITES,
  MIN_SERVE_PORT,
  type Locale,
  type McpActivity,
  type McpHistoryRange,
  type McpRead,
  type McpSettings,
  type McpSetupClient,
  type McpWrite,
  type TFunction,
  errorText,
  isMcpWrite,
  mcpActivityText,
  mcpAddress,
  mcpRangeText,
  mcpSetupCommand,
  relativeTime,
} from "@voltip/shared";
import {
  Badge,
  Button,
  Icon,
  Input,
  LampText,
  Select,
  SettingsPane,
  SettingsRows,
  SettingsSection,
  StatusRow,
  Toggle,
  cx,
  useBackend,
  useI18n,
  useNow,
  useUiState,
} from "@voltip/ui";
import { type ReactNode, useEffect, useId, useRef, useState } from "react";
import { resolvePlatform } from "../../app/platform";
import { copyWithToast, useShell } from "../../app/shell-context";
import { validPort } from "./ServicePane";

/** How many lines of the recent activity show before 查看全部. */
export const MCP_ACTIVITY_SHOWN = 5;

const READ_LABEL = {
  rules: "settings.mcp.read.rules",
  summary: "settings.mcp.read.summary",
  full: "settings.mcp.read.full",
} as const satisfies Record<McpRead, string>;

const READ_HELP = {
  rules: "settings.mcp.read.rulesHelp",
  summary: "settings.mcp.read.summaryHelp",
  full: "settings.mcp.read.fullHelp",
} as const satisfies Record<McpRead, string>;

const WRITE_LABEL = {
  read_only: "settings.mcp.write.readOnly",
  dictionary: "settings.mcp.write.dictionary",
  enable_rules: "settings.mcp.write.rules",
} as const satisfies Record<McpWrite, string>;

const WRITE_HELP = {
  read_only: "settings.mcp.write.readOnlyHelp",
  dictionary: "settings.mcp.write.dictionaryHelp",
  enable_rules: "settings.mcp.write.rulesHelp",
} as const satisfies Record<McpWrite, string>;

const SETUP_HELP = {
  claude_code: "settings.mcp.setup.claudeCodeHelp",
  codex: "settings.mcp.setup.codexHelp",
} as const satisfies Record<McpSetupClient, string>;

/** The levels the design recommends (and the settings start with). */
const RECOMMENDED_READ: McpRead = "summary";
const RECOMMENDED_WRITE: McpWrite = "dictionary";

function rangeOptions(t: TFunction) {
  return MCP_HISTORY_RANGES.map((value) => ({
    value,
    label: t(`settings.mcp.rangeOption.${value}`),
  }));
}

/** A list of names in a sentence: 「Claude Code、Codex」 / "Claude Code, Codex". */
function joined(items: readonly string[], locale: Locale): string {
  return items.join(locale === "zh-CN" ? "、" : ", ");
}

/** Settings · AI 工具接入 (docs/dictation.md §25): the local MCP service the AI tools on this
 *  computer (Claude Code, Codex) connect to — the switch, its status and address, the port; what the
 *  tools may read (the full history only after a confirmation that names the risk, with the period
 *  it covers) and change (turning rules on directly, likewise); the commands that connect each tool,
 *  shown with a placeholder: 复制命令 has the core put the command with the real token on the
 *  clipboard, so the token never reaches this page; the token; and the recent activity, where a
 *  write can be undone. */
export function McpPane() {
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const state = useUiState();
  const now = useNow();
  const mcp = state.settings.mcp;
  const status = state.mcp;
  const servePort = state.settings.serve.port;
  const [portText, setPortText] = useState(String(mcp.port));
  const [portTouched, setPortTouched] = useState(false);
  const [showAll, setShowAll] = useState(false);
  // The writes 撤销 was pressed for: their toast waits until the core reports them undone.
  const undoing = useRef(new Set<string>());
  const port = validPort(portText);

  const fail = (e: unknown) => {
    shell.toast({ message: t("common.errorPrefix", { message: errorText(e) }), duration: 5000 });
  };
  const send = (patch: Partial<McpSettings>) => {
    const next = { ...mcp, ...patch };
    return backend.invoke("settings_set_mcp", {
      enabled: next.enabled,
      port: next.port,
      read: next.read,
      historyRange: next.history_range,
      write: next.write,
    });
  };
  const set = (patch: Partial<McpSettings>) => {
    send(patch).catch(fail);
  };

  const portError = !portTouched
    ? undefined
    : port === undefined
      ? t("settings.mcp.portInvalid", { min: MIN_SERVE_PORT })
      : port === servePort
        ? t("settings.mcp.portTaken")
        : undefined;
  const applyPort = () => {
    setPortTouched(true);
    if (port !== undefined && port !== servePort && port !== mcp.port) set({ port });
  };

  // 完整历史 asks first, with the period it covers; until then the choice stays where it was.
  const chooseRead = (read: McpRead) => {
    if (read === mcp.read) return;
    if (read !== "full") {
      set({ read });
      return;
    }
    const choice = { range: mcp.history_range };
    shell.confirm({
      title: t("settings.mcp.confirmFull.title"),
      body: (
        <FullHistoryConfirm
          initial={mcp.history_range}
          onChange={(range) => {
            choice.range = range;
          }}
        />
      ),
      confirmLabel: t("settings.mcp.confirmFull.allow"),
      tone: "primary",
      onConfirm: () => {
        set({ read: "full", history_range: choice.range });
      },
    });
  };
  const revoke = () => {
    send({ read: RECOMMENDED_READ }).then(() => {
      shell.toast({ message: t("settings.mcp.revoked"), duration: 3000 });
    }, fail);
  };
  // So does letting the tools turn rules on: a rule changes the text inserted into other apps.
  const chooseWrite = (write: McpWrite) => {
    if (write === mcp.write) return;
    if (write !== "enable_rules") {
      set({ write });
      return;
    }
    shell.confirm({
      title: t("settings.mcp.confirmRules.title"),
      body: t("settings.mcp.confirmRules.body"),
      confirmLabel: t("settings.mcp.confirmRules.allow"),
      tone: "primary",
      onConfirm: () => {
        set({ write });
      },
    });
  };

  const copySetup = (client: McpSetupClient) => {
    backend.invoke("mcp_copy_setup", { client }).then(() => {
      shell.toast({ message: t("settings.mcp.setup.copied"), duration: 3000 });
    }, fail);
  };
  const copyToken = () => {
    backend.invoke("mcp_copy_token").then(() => {
      shell.toast({ message: t("settings.mcp.tokenCopied"), duration: 3000 });
    }, fail);
  };
  const rotate = () => {
    shell.confirm({
      title: t("settings.mcp.rotateConfirmTitle"),
      body: t("settings.mcp.rotateConfirmBody"),
      confirmLabel: t("settings.mcp.rotateConfirm"),
      tone: "danger",
      onConfirm: () => {
        backend.invoke("mcp_rotate_token").then(() => {
          shell.toast({ message: t("settings.mcp.rotated"), duration: 3000 });
        }, fail);
      },
    });
  };
  const undo = (activity: McpActivity) => {
    undoing.current.add(activity.id);
    backend.invoke("mcp_undo", { id: activity.id }).catch((e: unknown) => {
      undoing.current.delete(activity.id);
      fail(e);
    });
  };
  // The core refuses an undo it cannot do (the item changed since) with an error toast of its own;
  // this one says the undo happened, once the activity shows it.
  useEffect(() => {
    for (const activity of status.activity) {
      if (!activity.undone || !undoing.current.has(activity.id)) continue;
      undoing.current.delete(activity.id);
      shell.toast({
        message: t("settings.mcp.activity.undoDone", {
          summary: mcpActivityText(activity, locale),
        }),
        duration: 3000,
      });
    }
  }, [status.activity, undoing, shell, t, locale]);

  const running = status.phase === "running";
  const statusText = running
    ? t("settings.mcp.statusRunning")
    : status.phase === "starting"
      ? t("settings.mcp.statusStarting")
      : status.phase === "failed"
        ? t("settings.mcp.statusFailed", { reason: status.error ?? "" })
        : t("settings.mcp.statusOff");
  const tone = running
    ? "ok"
    : status.phase === "starting"
      ? "warn"
      : status.phase === "failed"
        ? "danger"
        : "off";
  const clients = joined(
    status.clients.map((client) =>
      t("settings.mcp.clientSeen", {
        name: client.name,
        time: relativeTime(Math.floor(client.last_seen_ms / 1000), now, locale),
      }),
    ),
    locale,
  );
  const statusHelp = running ? (
    status.clients.length > 0 ? (
      <span data-user-text data-testid="mcp-clients">
        {t("settings.mcp.connected", { clients })}
      </span>
    ) : (
      <span data-testid="mcp-clients">{t("settings.mcp.noClient")}</span>
    )
  ) : undefined;
  // What the tools connect to: the running listener's, or the one the saved port will give.
  const address = running && status.address !== undefined ? status.address : mcpAddress(mcp.port);
  const windows = resolvePlatform(state.identity?.platform) === "windows";
  const placeholder = t("settings.mcp.setup.tokenPlaceholder");
  const activity = showAll ? status.activity : status.activity.slice(0, MCP_ACTIVITY_SHOWN);

  return (
    <SettingsPane
      title={t("settings.mcp.title")}
      lede={t("settings.mcp.body")}
      actions={
        <Button
          size="sm"
          variant="ghost"
          icon="external"
          data-testid="mcp-guide"
          onClick={() => {
            backend.guideOpen("mcp", locale).catch(fail);
          }}>
          {t("settings.mcp.guide")}
        </Button>
      }
      data-testid="mcp-pane">
      <SettingsSection title={t("settings.mcp.serviceTitle")}>
        <SettingsRows>
          <StatusRow label={t("settings.mcp.enable")} help={t("settings.mcp.enableHelp")}>
            <Toggle
              checked={mcp.enabled}
              ariaLabel={t("settings.mcp.enable")}
              onChange={(enabled) => {
                set({ enabled });
              }}
            />
          </StatusRow>
          <StatusRow label={t("settings.mcp.status")} help={statusHelp}>
            <div className="flex min-w-0 flex-wrap items-center justify-end gap-x-3 gap-y-1">
              <LampText tone={tone} size="sm">
                <span
                  data-testid="mcp-status"
                  data-user-text={status.phase === "failed" ? "" : undefined}>
                  {statusText}
                </span>
              </LampText>
              {running && status.address !== undefined && (
                <>
                  <span className="mono truncate text-[12px] text-fg" data-testid="mcp-address">
                    {status.address}
                  </span>
                  <Button
                    size="sm"
                    variant="ghost"
                    icon="copy"
                    data-testid="mcp-copy-address"
                    onClick={() => {
                      void copyWithToast(shell, address, t("settings.mcp.addressCopied"));
                    }}>
                    {t("settings.mcp.copyAddress")}
                  </Button>
                </>
              )}
            </div>
          </StatusRow>
          <StatusRow
            label={t("settings.mcp.port")}
            help={t("settings.mcp.portHelp", { min: MIN_SERVE_PORT })}>
            <div className="flex items-start gap-2">
              <Input
                size="sm"
                mono
                inputMode="numeric"
                aria-label={t("settings.mcp.port")}
                className="w-24"
                value={portText}
                data-testid="mcp-port"
                error={portError}
                onChange={(e) => {
                  setPortText(e.target.value);
                  setPortTouched(false);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") applyPort();
                }}
              />
              <Button
                size="sm"
                variant="outline"
                data-testid="mcp-port-apply"
                disabled={port === mcp.port}
                onClick={applyPort}>
                {t("settings.mcp.portApply")}
              </Button>
            </div>
          </StatusRow>
        </SettingsRows>
      </SettingsSection>

      <SettingsSection title={t("settings.mcp.readTitle")} description={t("settings.mcp.readLede")}>
        <RadioList
          label={t("settings.mcp.readTitle")}
          value={mcp.read}
          onChange={chooseRead}
          data-testid="mcp-read"
          options={MCP_READS.map((value) => ({
            value,
            label: t(READ_LABEL[value]),
            help: t(READ_HELP[value]),
            recommended: value === RECOMMENDED_READ,
          }))}
          extra={
            mcp.read === "full" ? (
              <div className="flex flex-col gap-2" data-testid="mcp-full">
                <RangeSelect
                  value={mcp.history_range}
                  data-testid="mcp-range"
                  onChange={(history_range) => {
                    set({ history_range });
                  }}
                />
                <div
                  role="status"
                  data-testid="mcp-full-warning"
                  className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded-10 bg-warning-soft px-3 py-2 text-[12px] text-warning">
                  <Icon name="alert" size={14} className="shrink-0" />
                  <span>
                    {t("settings.mcp.read.fullWarning", {
                      range: mcpRangeText(mcp.history_range, locale),
                    })}
                  </span>
                  <Button size="sm" variant="text" data-testid="mcp-revoke" onClick={revoke}>
                    {t("settings.mcp.revoke")}
                  </Button>
                </div>
              </div>
            ) : undefined
          }
        />
      </SettingsSection>

      <SettingsSection title={t("settings.mcp.writeTitle")}>
        <RadioList
          label={t("settings.mcp.writeTitle")}
          value={mcp.write}
          onChange={chooseWrite}
          data-testid="mcp-write"
          options={MCP_WRITES.map((value) => ({
            value,
            label: t(WRITE_LABEL[value]),
            help: t(WRITE_HELP[value]),
            recommended: value === RECOMMENDED_WRITE,
          }))}
        />
        <p className="text-[12px] leading-4 text-fg-muted">{t("settings.mcp.writeNote")}</p>
      </SettingsSection>

      <SettingsSection title={t("settings.mcp.setupTitle")}>
        <SettingsRows>
          {MCP_SETUP_CLIENTS.map((client) => (
            <div
              key={client}
              className="flex flex-col gap-2 border-b border-border py-3"
              data-testid={`mcp-setup-${client}`}>
              <div className="flex items-start justify-between gap-4">
                <div className="min-w-0">
                  <div className="text-[14px] font-medium text-fg">{MCP_SETUP_LABELS[client]}</div>
                  <div className="mt-0.5 text-[12px] leading-4 text-fg-muted">
                    {t(SETUP_HELP[client])}
                  </div>
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  icon="copy"
                  aria-label={`${t("settings.mcp.setup.copy")} · ${MCP_SETUP_LABELS[client]}`}
                  data-testid={`mcp-copy-setup-${client}`}
                  onClick={() => {
                    copySetup(client);
                  }}>
                  {t("settings.mcp.setup.copy")}
                </Button>
              </div>
              <pre
                className="mono rounded-10 border border-border bg-inset px-3 py-2 text-[11px] leading-5 break-words whitespace-pre-wrap text-fg"
                data-testid={`mcp-setup-command-${client}`}>
                {mcpSetupCommand(client, address, placeholder, windows)}
              </pre>
            </div>
          ))}
        </SettingsRows>
        <p className="text-[12px] leading-4 text-fg-muted">{t("settings.mcp.setup.note")}</p>
      </SettingsSection>

      <SettingsSection title={t("settings.mcp.tokenTitle")}>
        <SettingsRows>
          <StatusRow label={t("settings.mcp.token")} help={t("settings.mcp.tokenHelp")}>
            <div className="flex items-center gap-2">
              <Button
                size="sm"
                variant="outline"
                icon="copy"
                data-testid="mcp-copy-token"
                onClick={copyToken}>
                {t("settings.mcp.copyToken")}
              </Button>
              <Button size="sm" variant="ghost" data-testid="mcp-rotate-token" onClick={rotate}>
                {t("settings.mcp.rotate")}
              </Button>
            </div>
          </StatusRow>
        </SettingsRows>
      </SettingsSection>

      <SettingsSection
        title={t("settings.mcp.activity.title")}
        aside={
          status.activity.length > MCP_ACTIVITY_SHOWN ? (
            <Button
              size="sm"
              variant="text"
              aria-expanded={showAll}
              data-testid="mcp-activity-toggle"
              onClick={() => {
                setShowAll(!showAll);
              }}>
              {showAll ? t("settings.mcp.activity.showLess") : t("settings.mcp.activity.showAll")}
            </Button>
          ) : undefined
        }>
        {status.activity.length === 0 ? (
          <p className="text-[12px] leading-4 text-fg-muted" data-testid="mcp-activity-empty">
            {t("settings.mcp.activity.empty")}
          </p>
        ) : (
          <ul
            className="border-t border-border"
            aria-label={t("settings.mcp.activity.title")}
            data-testid="mcp-activity">
            {activity.map((item) => (
              <ActivityRow
                key={item.id}
                activity={item}
                now={now}
                locale={locale}
                t={t}
                onUndo={() => {
                  undo(item);
                }}
              />
            ))}
          </ul>
        )}
      </SettingsSection>
    </SettingsPane>
  );
}

/** The confirmation's body: what the tools could read, the risk, and the period it covers (the
 *  saved one to start with; 允许读取 takes what is chosen here). */
function FullHistoryConfirm({
  initial,
  onChange,
}: {
  initial: McpHistoryRange;
  onChange: (range: McpHistoryRange) => void;
}) {
  const { t } = useI18n();
  const [range, setRange] = useState(initial);
  return (
    <div className="flex flex-col gap-3" data-testid="mcp-confirm-full">
      <p>{t("settings.mcp.confirmFull.body")}</p>
      <p>{t("settings.mcp.confirmFull.risk")}</p>
      <RangeSelect
        strong
        value={range}
        data-testid="mcp-confirm-range"
        onChange={(next) => {
          setRange(next);
          onChange(next);
        }}
      />
    </div>
  );
}

/** 时间范围 beside its select, the label naming it. */
function RangeSelect({
  value,
  onChange,
  strong = false,
  "data-testid": testId,
}: {
  value: McpHistoryRange;
  onChange: (range: McpHistoryRange) => void;
  strong?: boolean;
  "data-testid"?: string;
}) {
  const { t } = useI18n();
  const id = useId();
  return (
    <div className="flex items-center gap-3">
      <label htmlFor={id} className={cx("text-[13px] text-fg", strong && "font-medium")}>
        {t("settings.mcp.range")}
      </label>
      <Select
        id={id}
        size="sm"
        value={value}
        data-testid={testId}
        options={rangeOptions(t)}
        onChange={onChange}
      />
    </div>
  );
}

interface RadioOption<V extends string> {
  value: V;
  label: string;
  help: string;
  recommended: boolean;
}

/** A vertical radio group whose options carry a line of help; the chosen option can show more
 *  under it (`extra`). */
function RadioList<V extends string>({
  label,
  value,
  options,
  onChange,
  extra,
  "data-testid": testId,
}: {
  label: string;
  value: V;
  options: readonly RadioOption<V>[];
  onChange: (value: V) => void;
  extra?: ReactNode;
  "data-testid"?: string;
}) {
  const { t } = useI18n();
  const id = useId();
  return (
    <div role="radiogroup" aria-label={label} className="flex flex-col gap-1" data-testid={testId}>
      {options.map((option) => {
        const checked = option.value === value;
        return (
          <div key={option.value} className="flex flex-col gap-2">
            <button
              type="button"
              role="radio"
              aria-checked={checked}
              aria-labelledby={`${id}-${option.value}`}
              aria-describedby={`${id}-${option.value}-help`}
              data-value={option.value}
              onClick={() => {
                onChange(option.value);
              }}
              className="flex items-start gap-3 rounded-10 px-2 py-2 text-left transition-colors hover:bg-inset">
              <span
                aria-hidden
                className={cx(
                  "mt-0.5 inline-block h-4 w-4 shrink-0 rounded-full border bg-field",
                  checked ? "border-[5px] border-primary" : "border-border-strong",
                )}
              />
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-2 text-[14px] font-medium text-fg">
                  <span id={`${id}-${option.value}`}>{option.label}</span>
                  {option.recommended && (
                    <Badge tone="accent">{t("settings.mcp.recommended")}</Badge>
                  )}
                </span>
                <span
                  id={`${id}-${option.value}-help`}
                  className="mt-0.5 block text-[12px] leading-4 text-fg-muted">
                  {option.help}
                </span>
              </span>
            </button>
            {checked && extra !== undefined && <div className="pl-9">{extra}</div>}
          </div>
        );
      })}
    </div>
  );
}

/** One line of the recent activity: what was read or changed, when, and 撤销 for a write. */
function ActivityRow({
  activity,
  now,
  locale,
  t,
  onUndo,
}: {
  activity: McpActivity;
  now: number;
  locale: Locale;
  t: TFunction;
  onUndo: () => void;
}) {
  const write = isMcpWrite(activity);
  const text = mcpActivityText(activity, locale);
  return (
    <li
      className="flex min-h-11 items-center gap-3 border-b border-border py-2 last:border-b-0"
      data-testid="mcp-activity-row"
      data-action={activity.action}>
      <Icon
        name={write ? "edit" : "eye"}
        size={16}
        aria-label={write ? t("settings.mcp.activity.wrote") : t("settings.mcp.activity.read")}
        role="img"
        className={cx("shrink-0", write ? "text-accent-text" : "text-fg-subtle")}
      />
      <span
        className={cx(
          "min-w-0 flex-1 truncate text-[13px]",
          activity.undone ? "text-fg-subtle line-through" : "text-fg",
        )}
        data-user-text
        title={text}>
        {text}
      </span>
      <span className="mono shrink-0 text-[11px] text-fg-subtle">
        {relativeTime(Math.floor(activity.at_ms / 1000), now, locale)}
      </span>
      {activity.undone ? (
        <span className="shrink-0 text-[12px] text-fg-subtle">
          {t("settings.mcp.activity.undone")}
        </span>
      ) : activity.undoable ? (
        <Button
          size="sm"
          variant="ghost"
          aria-label={t("settings.mcp.activity.undoLabel", { summary: text })}
          onClick={onUndo}>
          {t("settings.mcp.activity.undo")}
        </Button>
      ) : null}
    </li>
  );
}
