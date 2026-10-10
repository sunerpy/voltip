import {
  ProviderCards as SharedProviderCards,
  type ProviderCardsProps as SharedProviderCardsProps,
} from "@voltip/ui";
import { LocalCompute } from "./LocalCompute";
import { LocalModels } from "./LocalModels";

export type ProviderCardsProps = Omit<SharedProviderCardsProps, "localBody">;

/** The shared provider cards (`@voltip/ui`, also on the phone) with the desktop's model library
 *  and compute settings in the on-device provider's drawer. */
export function ProviderCards(props: ProviderCardsProps) {
  return (
    <SharedProviderCards
      {...props}
      localBody={
        <>
          <LocalModels />
          <LocalCompute />
        </>
      }
    />
  );
}
