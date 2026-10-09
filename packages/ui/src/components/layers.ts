/** What is open over the page, the most recent last: dialogs, and a touch `Select`'s list of
 *  options. Esc and the phone's system back close only the layer on top: a confirmation over an
 *  editor closes and the editor stays; a list opened in a dialog closes and the dialog stays. */
const layers: { close: () => void }[] = [];

export interface Layer {
  /** No layer opened after this one is still open. */
  isTop(): boolean;
  /** Takes the layer off the stack once it has closed. */
  remove(): void;
}

/** Puts a layer on top of the others; `close` is what Esc or the system back do to it. */
export function openLayer(close: () => void): Layer {
  const entry = { close };
  layers.push(entry);
  return {
    isTop: () => layers.at(-1) === entry,
    remove: () => {
      const at = layers.indexOf(entry);
      if (at >= 0) layers.splice(at, 1);
    },
  };
}

/** Closes the layer on top as Esc does; `false` when none is open. */
export function dismissTopLayer(): boolean {
  const top = layers.at(-1);
  if (top === undefined) return false;
  top.close();
  return true;
}
