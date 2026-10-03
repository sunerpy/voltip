import { type ReactNode, createContext, useContext } from "react";

/** How the controls are drawn and driven. `native` (the default, the desktop): `Select` is the
 *  platform's `<select>`. `touch` (the phone): `Select` opens its own list of options instead of
 *  Android's picker, and the controls take touch states (no tap flash, a pressed state, targets of
 *  at least 44 × 44 px). */
export type Presentation = "native" | "touch";

const PresentationContext = createContext<Presentation>("native");

export interface PresentationProviderProps {
  value: Presentation;
  children: ReactNode;
}

/** Sets the presentation for every control under it; without a provider it is `native`, so the
 *  desktop renders exactly as before. */
export function PresentationProvider({ value, children }: PresentationProviderProps) {
  return <PresentationContext.Provider value={value}>{children}</PresentationContext.Provider>;
}

/** The presentation of the nearest `PresentationProvider` (`native` without one). */
export function usePresentation(): Presentation {
  return useContext(PresentationContext);
}
