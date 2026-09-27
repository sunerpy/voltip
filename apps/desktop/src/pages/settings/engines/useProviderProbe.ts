import type { ProbeReport, ProviderId, ServiceKind } from "@voltip/shared";
import { useBackend } from "@voltip/ui";
import { useCallback, useEffect, useRef, useState } from "react";

/** How long the pane waits for a `provider_probe` answer before it says the test timed out (the
 *  core's own request deadline is 10 s, so an answer normally comes first). */
export const PROBE_WAIT_MS = 20_000;

export interface ProviderProbe {
  /** A test is in flight. */
  pending: boolean;
  /** The last answer for this provider and service. */
  report: ProbeReport | undefined;
  /** Ask the core to list the provider's models with the form's values (`undefined` = saved). */
  run: (draft: { baseUrl?: string; key?: string }) => void;
}

/** 测试连接 for one provider card: sends `provider_probe` and listens for the `provider_probe`
 *  event that answers it. The key typed in the form goes out with this one request only. */
export function useProviderProbe(provider: ProviderId, kind: ServiceKind): ProviderProbe {
  const { backend } = useBackend();
  const [pending, setPending] = useState(false);
  const [report, setReport] = useState<ProbeReport | undefined>(undefined);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(
    () =>
      backend.on((event) => {
        if (event.type !== "provider_probe" || event.provider !== provider || event.kind !== kind)
          return;
        const { type: _type, ...answer } = event;
        clearTimeout(timer.current);
        setPending(false);
        setReport(answer);
      }),
    [backend, provider, kind],
  );
  useEffect(
    () => () => {
      clearTimeout(timer.current);
    },
    [],
  );

  const run = useCallback(
    (draft: { baseUrl?: string; key?: string }) => {
      const baseUrl = draft.baseUrl?.trim();
      const key = draft.key?.trim();
      setPending(true);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        setPending(false);
        setReport({ provider, kind, result: "failed", reason: "timeout" });
      }, PROBE_WAIT_MS);
      void backend
        .invoke("provider_probe", {
          provider,
          kind,
          baseUrl: baseUrl === undefined || baseUrl.length === 0 ? null : baseUrl,
          key: key === undefined || key.length === 0 ? null : key,
        })
        .catch(() => {
          clearTimeout(timer.current);
          setPending(false);
          setReport({ provider, kind, result: "failed", reason: "unsupported" });
        });
    },
    [backend, provider, kind],
  );

  return { pending, report, run };
}
