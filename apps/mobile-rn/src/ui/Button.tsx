// Paper's Button with the app's corner (docs/mobile-rn.md §5, the "简约中性" style): 12 dp, like
// the cards and fields, instead of Material's full pill. Everything else is Paper's.
import type { ComponentProps } from "react";
import { Button as PaperButton } from "react-native-paper";

/** The corner of a button. */
export const BUTTON_RADIUS = 12;

export function Button({ style, ...props }: ComponentProps<typeof PaperButton>) {
  return <PaperButton {...props} style={[{ borderRadius: BUTTON_RADIUS }, style]} />;
}
