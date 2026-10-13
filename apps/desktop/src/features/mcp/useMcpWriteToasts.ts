import { type McpActivity, isMcpWrite, mcpWritesToast } from "@voltip/shared";
import { useBackend, useI18n } from "@voltip/ui";
import { useEffect, useRef } from "react";
import { type Route, useRouter } from "../../app/router";
import { useShell } from "../../app/shell-context";

/** How long a write's toast stays (a 5 s toast, as for other notices with an action). */
export const MCP_TOAST_MS = 5000;

/** The page a toast's 查看 opens: the rules when a rule was written (one may wait for approval),
 *  the dictionary otherwise. */
export function routeForWrites(writes: readonly McpActivity[]): Route {
  return writes.some((w) => w.action === "add_rule" || w.action === "update_rule")
    ? { name: "rules" }
    : { name: "dictionary" };
}

/** A toast for what an AI tool wrote into the dictionary or the rules (docs/dictation.md §25):
 *  「Claude Code 添加了 2 个词条」 with 查看. The tool's next writes join its toast while that is
 *  still shown. The activity the core had before this window saw it (the log survives restarts)
 *  is never announced: the first status of a service that can run is taken as seen. */
export function useMcpWriteToasts(): void {
  const { state } = useBackend();
  const status = state?.mcp;
  const shell = useShell();
  const { navigate } = useRouter();
  const { t, locale } = useI18n();
  // The activity ids already looked at; `undefined` until the core's first real status.
  const seen = useRef<Set<string> | undefined>(undefined);
  // The toast on screen for the last client that wrote, with what it said so far.
  const shown = useRef<{ client: string; id: string; writes: McpActivity[]; until: number }>(
    undefined,
  );

  useEffect(() => {
    if (status === undefined || !status.available) return;
    const ids = new Set(status.activity.map((a) => a.id));
    const before = seen.current;
    // Only what is still listed can come back, so the set never outgrows the log.
    seen.current = ids;
    if (before === undefined) return;
    // The log is newest first; the toasts follow the order of the writes, oldest first.
    const byClient = new Map<string, McpActivity[]>();
    for (let at = status.activity.length - 1; at >= 0; at -= 1) {
      const write = status.activity[at];
      if (write === undefined || before.has(write.id) || !isMcpWrite(write) || write.undone)
        continue;
      byClient.set(write.client, [...(byClient.get(write.client) ?? []), write]);
    }
    for (const [client, writes] of byClient) {
      const now = Date.now();
      const last = shown.current;
      const joins = last !== undefined && last.client === client && now < last.until;
      if (joins) shell.toasts.dismiss(last.id);
      const all = joins ? [...last.writes, ...writes] : writes;
      const id = shell.toast({
        message: mcpWritesToast(client, all, locale),
        duration: MCP_TOAST_MS,
        action: {
          label: t("mcp.toast.view"),
          onClick: () => {
            navigate(routeForWrites(all));
          },
        },
      });
      shown.current = { client, id, writes: all, until: now + MCP_TOAST_MS };
    }
  }, [status, shell, navigate, t, locale]);
}
