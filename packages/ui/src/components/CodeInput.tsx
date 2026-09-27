import { useId, useRef } from "react";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";

export interface CodeInputProps {
  value: string;
  onChange: (digits: string) => void;
  /** Fires once when the sixth digit lands. */
  onComplete?: (digits: string) => void;
  disabled?: boolean;
  error?: string;
  label?: string;
  autoFocus?: boolean;
}

export const CODE_LENGTH = 6;

/** Six-digit pairing code entry drawn as `_ _ _   _ _ _`; one real input drives the display cells. */
export function CodeInput({
  value,
  onChange,
  onComplete,
  disabled = false,
  error,
  label,
  autoFocus = false,
}: CodeInputProps) {
  const t = useT();
  const id = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const digits = value.replace(/\D/g, "").slice(0, CODE_LENGTH);
  return (
    <div className="flex flex-col gap-2">
      <label htmlFor={id} className="sr-only">
        {label ?? t("ui.a11y.codeInput")}
      </label>
      <div
        className="relative flex items-center justify-center gap-2"
        onClick={() => {
          inputRef.current?.focus();
        }}>
        {Array.from({ length: CODE_LENGTH }, (_, i) => (
          <span key={i} className="flex items-center">
            {i === 3 && <span className="w-4" aria-hidden />}
            <span
              data-cell={i}
              data-filled={i < digits.length ? "true" : "false"}
              className={cx(
                "mono flex h-14 w-11 items-center justify-center rounded-10 bg-surface text-[28px] hairline",
                i === digits.length && !disabled && "border-fg shadow-[0_0_0_1px_var(--fg)]",
                error && "border-danger",
              )}>
              {digits[i] ?? <span className="text-fg-subtle">_</span>}
            </span>
          </span>
        ))}
        <input
          ref={inputRef}
          id={id}
          inputMode="numeric"
          autoComplete="one-time-code"
          pattern="[0-9]*"
          maxLength={CODE_LENGTH}
          value={digits}
          disabled={disabled}
          autoFocus={autoFocus}
          aria-invalid={error ? true : undefined}
          onChange={(e) => {
            const next = e.target.value.replace(/\D/g, "").slice(0, CODE_LENGTH);
            onChange(next);
            if (next.length === CODE_LENGTH && next !== digits) onComplete?.(next);
          }}
          className="absolute inset-0 h-full w-full cursor-text opacity-0"
        />
      </div>
      {error && (
        <p role="alert" className="text-center text-[12px] text-danger">
          {error}
        </p>
      )}
    </div>
  );
}
