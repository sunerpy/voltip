import type { Backend, ProjectLink } from "@voltip/shared";
import { errorText } from "../features/vocabulary/vocabulary";
import type { ShellActions } from "./shell-context";

/** Open a project page (`project_link_open`); a refusal (no browser, no opener) becomes a toast
 *  instead of a click that silently does nothing. */
export function openProjectLink(
  backend: Backend,
  shell: Pick<ShellActions, "toast">,
  link: ProjectLink,
): void {
  backend.projectLinkOpen(link).catch((e: unknown) => {
    shell.toast({ message: errorText(e), duration: 5000 });
  });
}
