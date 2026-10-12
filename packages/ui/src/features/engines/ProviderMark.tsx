import { PROVIDER_MARKS, type ProviderId } from "@voltip/shared";

/** A vendor's mark (`PROVIDER_MARKS`, docs/dictation.md §3) at `size` px; nothing for a provider
 *  without one. Decorative: the provider's name is beside it. A path without its own colour takes
 *  the text colour, so the monochrome marks follow the theme. */
export function ProviderMark({ id, size }: { id: ProviderId; size: number }) {
  const mark = PROVIDER_MARKS[id];
  if (mark === undefined) return null;
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="currentColor"
      aria-hidden
      data-mark={id}
      className="shrink-0">
      {mark.paths.map((path) => (
        <path
          key={path.d}
          d={path.d}
          fill={path.fill}
          fillRule={path.evenOdd === true ? "evenodd" : undefined}
          clipRule={path.evenOdd === true ? "evenodd" : undefined}
        />
      ))}
    </svg>
  );
}

/** Whether `id` has a mark of its own. */
export function hasProviderMark(id: ProviderId): boolean {
  return PROVIDER_MARKS[id] !== undefined;
}
