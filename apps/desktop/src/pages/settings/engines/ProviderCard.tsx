import {
  ProviderCard as SharedProviderCard,
  type ProviderCardProps as SharedProviderCardProps,
} from "@voltip/ui";
import { LocalCompute } from "./LocalCompute";
import { LocalModels } from "./LocalModels";

export type ProviderCardProps = Omit<SharedProviderCardProps, "localBody">;

/** The shared provider card (`@voltip/ui`, also on the phone) with the desktop's model library as
 *  the on-device card's body. */
export function ProviderCard(props: ProviderCardProps) {
  return (
    <SharedProviderCard
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
