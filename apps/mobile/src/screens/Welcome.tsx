import { Button, Lamp, Logo, useT } from "@voltip/ui";
import { useMobileShell } from "../app/shell";
import { PhoneMic } from "./PhoneMic";
import { RecentResults } from "./RecentResults";

const POINTS = ["e2ee", "safety", "identity"] as const;

/** The first screen while no computer is paired: talk on the phone right away (docs/dictation.md
 *  §20.7), and what pairing a computer adds. */
export function Welcome() {
  const shell = useMobileShell();
  const t = useT();
  return (
    <div className="flex flex-col gap-4 p-4">
      <div className="mt-6 flex flex-col items-center gap-3 text-center">
        <Logo size={44} label="Voltip" />
        <h1 className="text-[24px] font-semibold text-fg">Voltip</h1>
        <p className="text-[14px] leading-6 text-fg-muted">{t("mobile.welcome.intro")}</p>
      </div>
      <PhoneMic desktops={[]} />
      <RecentResults />
      <section className="flex flex-col gap-3" aria-labelledby="welcome-pairing">
        <div className="flex flex-col gap-1">
          <h2 id="welcome-pairing" className="text-[15px] font-semibold text-fg">
            {t("mobile.welcome.pairing")}
          </h2>
          <p className="text-[12px] leading-5 text-fg-muted">{t("mobile.welcome.pairingBody")}</p>
        </div>
        <ul className="flex flex-col gap-3">
          {POINTS.map((point) => (
            <li key={point} className="flex gap-3 rounded-10 bg-surface p-4 hairline">
              <Lamp tone="ok" className="mt-1.5" />
              <div>
                <div className="text-[14px] font-medium text-fg">
                  {t(`mobile.welcome.${point}`)}
                </div>
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
      </section>
    </div>
  );
}
