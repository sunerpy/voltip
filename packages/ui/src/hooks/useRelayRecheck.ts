import type { Backend } from "@voltip/shared";
import { useEffect } from "react";

/** docs/pairing.md 「重连」: the browser says the network is back, or the window came to the front.
 *  A relay socket from before is likely dead, so the core's relay link checks it now
 *  (`relay_reconnect`) instead of waiting for its heartbeat. One window asks: the main one. */
export function useRelayRecheck(backend: Backend) {
  useEffect(() => {
    const check = () => {
      backend.invoke("relay_reconnect").catch(() => {});
    };
    const shown = () => {
      if (document.visibilityState === "visible") check();
    };
    window.addEventListener("online", check);
    document.addEventListener("visibilitychange", shown);
    return () => {
      window.removeEventListener("online", check);
      document.removeEventListener("visibilitychange", shown);
    };
  }, [backend]);
}
