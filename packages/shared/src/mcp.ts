// The local MCP service's helpers the pages share (docs/dictation.md §25): the commands 接入 shows
// (the core copies the same text with the real token: `voltip_core::mcp::setup_command`, kept in
// step through fixtures/ipc/mcp-setup.json), the recent activity as one line of text, and the rules
// an AI tool created that wait for the user's approval.
import { DEFAULT_LOCALE, type Locale, type MessageKey, translate } from "./i18n";
import {
  MCP_WRITE_ACTIONS,
  type McpActivity,
  type McpHistoryRange,
  type McpSetupClient,
  type ReplacementRule,
} from "./schema";

/** The name the tools register the service under (`voltip_core::mcp::setup::SERVER_NAME`). */
export const MCP_SERVER_NAME = "voltip";
/** The environment variable Codex reads the token from (`TOKEN_ENV_VAR`). */
export const MCP_TOKEN_ENV_VAR = "VOLTIP_MCP_TOKEN";
/** What 接入 calls each tool (product names, the same in every language). */
export const MCP_SETUP_LABELS: Readonly<Record<McpSetupClient, string>> = {
  claude_code: "Claude Code",
  codex: "Codex",
};

/** `http://127.0.0.1:<port>/mcp` (`voltip_core::mcp::address`). */
export function mcpAddress(port: number): string {
  return `http://127.0.0.1:${port}/mcp`;
}

/** The command that connects `client` to `address` with `token` — on Windows the environment
 *  variable is set the PowerShell way. */
export function mcpSetupCommand(
  client: McpSetupClient,
  address: string,
  token: string,
  windows: boolean,
): string {
  if (client === "claude_code") {
    return `claude mcp add --transport http ${MCP_SERVER_NAME} ${address} --header "Authorization: Bearer ${token}"`;
  }
  const set = windows
    ? `$env:${MCP_TOKEN_ENV_VAR} = "${token}"`
    : `export ${MCP_TOKEN_ENV_VAR}=${token}`;
  return `${set}\ncodex mcp add ${MCP_SERVER_NAME} --url ${address} --bearer-token-env-var ${MCP_TOKEN_ENV_VAR}`;
}

/** Whether the activity changed the dictionary or the rules. */
export function isMcpWrite(activity: Pick<McpActivity, "action">): boolean {
  return MCP_WRITE_ACTIONS.includes(activity.action);
}

/** A history range inside a sentence (「最近 30 天」 / "the last 30 days"). */
export function mcpRangeText(range: McpHistoryRange, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, `settings.mcp.rangeText.${range}` as MessageKey);
}

/** One line of the recent activity: who did what, about what, how many. */
export function mcpActivityText(activity: McpActivity, locale: Locale = DEFAULT_LOCALE): string {
  const key = `settings.mcp.activity.action.${activity.action}` as MessageKey;
  const text = translate(locale, key, {
    client: activity.client,
    subject: activity.subject ?? "",
    n: activity.count ?? 0,
    range: activity.range === undefined ? "" : mcpRangeText(activity.range, locale),
  });
  return activity.pending ? text + translate(locale, "settings.mcp.activity.pending") : text;
}

/** The rules an AI tool created that wait for the user's approval, in list order. */
export function pendingRules(rules: readonly ReplacementRule[]): ReplacementRule[] {
  return rules.filter((rule) => rule.pending);
}

/** Who created the waiting rules: one name when it is the same tool, `undefined` for several. */
export function pendingRulesClient(rules: readonly ReplacementRule[]): string | undefined {
  const names = new Set(
    pendingRules(rules).flatMap((rule) => (rule.source.kind === "mcp" ? [rule.source.client] : [])),
  );
  return names.size === 1 ? [...names][0] : undefined;
}

/** What a toast says about the writes `fresh` (new since the last status, one client's): entries
 *  added, rules created that wait for approval, or changes in general. */
export function mcpWritesToast(
  client: string,
  fresh: readonly McpActivity[],
  locale: Locale = DEFAULT_LOCALE,
): string {
  const n = fresh.length;
  if (fresh.every((a) => a.action === "add_entry"))
    return translate(locale, "mcp.toast.wrote", { client, n });
  if (fresh.every((a) => a.action === "add_rule" && a.pending))
    return translate(locale, "mcp.toast.rules", { client, n });
  return translate(locale, "mcp.toast.changed", { client, n });
}
