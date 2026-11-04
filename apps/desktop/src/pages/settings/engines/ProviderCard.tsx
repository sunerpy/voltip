import {
  ProviderCards as SharedProviderCards,
  type ProviderCardsProps as SharedProviderCardsProps,
} from "@voltip/ui";
import { LocalCompute } from "./LocalCompute";
import { LocalModels } from "./LocalModels";

export type ProviderCardsProps = Omit<SharedProviderCardsProps, "localBody" | "previews">;

/** The shared provider cards (`@voltip/ui`, also on the phone) with the desktop's model library
 *  and compute settings in the on-device provider's drawer; the desktop shows live previews
 *  (docs/dictation.md §3.15). */
export function ProviderCards(props: ProviderCardsProps) {
  return (
    <SharedProviderCards
      {...props}
      previews
      localBody={
        <>
          <LocalModels />
          <LocalCompute />
        </>
      }
    />
  );
}
