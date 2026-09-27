import { Button, Lamp, Logo, useT } from "@voltip/ui";
import { useMobileShell } from "../app/shell";

const POINTS = ["e2ee", "safety", "identity"] as const;

export function Welcome() {
  const shell = useMobileShell();
  const t = useT();
  return (
    <div className="flex h-full flex-col justify-between p-6">
      <div className="mt-16 flex flex-col items-center gap-4 text-center">
        <Logo size={56} label="Voltip" />
        <h1 className="text-[28px] font-semibold text-fg">Voltip</h1>
        <p className="text-[14px] leading-6 text-fg-muted">{t("mobile.welcome.intro")}</p>
      </div>
      <ul className="flex flex-col gap-4">
        {POINTS.map((point) => (
          <li key={point} className="flex gap-3 rounded-10 bg-surface p-4 hairline">
            <Lamp tone="ok" className="mt-1.5" />
            <div>
              <div className="text-[14px] font-medium text-fg">{t(`mobile.welcome.${point}`)}</div>
              <div className="text-[12px] leading-5 text-fg-muted">
                {t(`mobile.welcome.${point}Body`)}
              </div>
            </div>
          </li>
        ))}
      </ul>
      <Button
        variant="primary"
        className="h-11 w-full text-[15px]"
        onClick={() => {
          shell.go("device");
        }}>
        {t("mobile.welcome.start")}
      </Button>
    </div>
  );
}
