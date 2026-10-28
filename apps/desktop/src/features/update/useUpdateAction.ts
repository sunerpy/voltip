import { useBackend, useUiState } from "@voltip/ui";
import { useShell } from "../../app/shell-context";

/** The way to the update the relay asks for (docs/pairing.md 「最低版本」): the update dialog,
 *  opened on a fresh check unless a newer version is already known. `install` also starts the
 *  download of a known version; the dialog then shows its progress. */
export function useUpdateAction(): {
  version: string | undefined;
  open: () => void;
  install: () => void;
} {
  const { backend } = useBackend();
  const shell = useShell();
  const { update } = useUiState();
  const version = update.state === "available" ? update.version : undefined;
  const open = () => {
    if (update.state === "idle" || update.state === "up_to_date" || update.state === "failed")
      void backend.invoke("update_check");
    shell.setUpdateOpen(true);
  };
  const install = () => {
    if (update.state === "available") {
      void backend.invoke("update_install");
      shell.setUpdateOpen(true);
      return;
    }
    open();
  };
  return { version, open, install };
}
