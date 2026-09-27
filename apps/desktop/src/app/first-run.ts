import { parseRoute } from "./router";

/** Set once the first-run guide has been finished or skipped (`pages/Onboarding.tsx` writes it). */
export const ONBOARDING_DONE_KEY = "voltip.onboarding.done";

/** Where a launch at `path` should start instead: the first-run guide while it has never been
 *  finished or skipped and the window opens at the home page; `undefined` = stay. Without storage
 *  the answer could not be remembered, so the guide is not forced on every launch. */
export function firstRunPath(
  path: string,
  storage: Pick<Storage, "getItem"> | undefined,
): string | undefined {
  if (storage === undefined || storage.getItem(ONBOARDING_DONE_KEY) !== null) return undefined;
  return parseRoute(path).name === "home" ? "/onboarding" : undefined;
}
