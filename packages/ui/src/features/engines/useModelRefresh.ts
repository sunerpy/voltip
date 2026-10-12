import type { ProviderId, ServiceKind } from "@voltip/shared";
import { useCallback } from "react";
import { useBackend } from "../../backend/BackendProvider";

/** Asks the core for a provider's latest model list (`provider_models_refresh`, docs/dictation.md
 *  §3.3), as a model list opens; the list arrives with the engines status
 *  (`ServiceStatus.listed`). The core lists with the saved endpoint and key only, one request at a
 *  time, and does nothing for the built-in and on-device providers or without a saved key. */
export function useModelRefresh(provider: ProviderId | undefined, kind: ServiceKind): () => void {
  const { backend } = useBackend();
  return useCallback(() => {
    if (provider === undefined) return;
    backend.invoke("provider_models_refresh", { provider, kind }).catch(() => undefined);
  }, [backend, provider, kind]);
}
