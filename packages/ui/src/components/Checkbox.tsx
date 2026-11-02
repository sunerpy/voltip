import { useEffect, useRef } from "react";
import { cx } from "../cx";

export interface CheckboxProps {
  checked: boolean;
  onChange: (next: boolean) => void;
  /** Accessible name (the box has no visible label of its own, as in a table row). */
  label: string;
  /** Some, not all, of what the box stands for are checked (全选 over a list): shown and read as
   *  mixed until it is checked or cleared. */
  indeterminate?: boolean;
  disabled?: boolean;
  className?: string;
}

/** A native check box, so Tab reaches it and Space toggles it. A click on it is its own: it does
 *  not reach a clickable row around it. */
export function Checkbox({
  checked,
  onChange,
  label,
  indeterminate = false,
  disabled = false,
  className,
}: CheckboxProps) {
  const box = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (box.current !== null) box.current.indeterminate = indeterminate;
  }, [indeterminate]);
  return (
    <input
      ref={box}
      type="checkbox"
      aria-label={label}
      checked={checked}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation();
      }}
      onChange={(e) => {
        onChange(e.target.checked);
      }}
      className={cx(
        "h-3.5 w-3.5 cursor-pointer align-middle accent-[var(--primary)] disabled:cursor-not-allowed disabled:opacity-40",
        className,
      )}
    />
  );
}
