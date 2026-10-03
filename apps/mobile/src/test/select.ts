import { screen, within } from "@testing-library/react";
import type { UserEvent } from "@testing-library/user-event";

/** The trigger of a `Select` under the phone's touch presentation (`App`'s `PresentationProvider`):
 *  a button named like the select, which opens the list of options. */
export function selectTrigger(name: string | RegExp, container: HTMLElement = document.body) {
  return within(container).getByRole("button", { name });
}

/** Choose `value` in a touch `Select`: open its list from the trigger and tap the option, as a
 *  finger does. */
export async function chooseOption(user: UserEvent, trigger: HTMLElement, value: string) {
  await user.click(trigger);
  await user.selectOptions(screen.getByRole("listbox"), value);
}
