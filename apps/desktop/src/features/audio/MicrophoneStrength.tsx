import type { LevelFrame } from "@voltip/shared";
import { LedMeter, cx, useI18n } from "@voltip/ui";
import { levelFraction } from "./useAudioMeter";

/** The strength bar with its dBFS and peak readouts: silent (0) while nothing meters. */
export function MicrophoneStrength({
  frame,
  disabled = false,
  className,
  "data-testid": testId,
}: {
  frame: LevelFrame | undefined;
  disabled?: boolean;
  className?: string;
  "data-testid"?: string;
}) {
  const { t } = useI18n();
  return (
    <div className={cx("flex items-end justify-between gap-4", className)} data-testid={testId}>
      <LedMeter
        level={frame ? levelFraction(frame.rms_dbfs) : 0}
        peak={frame ? levelFraction(frame.peak_dbfs) : undefined}
        disabled={disabled}
      />
      <div className="mono text-right text-[12px] leading-tight text-fg">
        <div>{frame ? `${frame.rms_dbfs.toFixed(1)} dBFS` : "— dBFS"}</div>
        <div className={frame?.clipping ? "text-danger" : "text-fg-muted"}>
          {t("home.mic.peak", { value: frame ? frame.peak_dbfs.toFixed(1) : "—" })}
        </div>
      </div>
    </div>
  );
}
