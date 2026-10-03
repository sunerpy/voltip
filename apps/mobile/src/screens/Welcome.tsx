import { Button, Card, Icon, type IconName, useT } from "@voltip/ui";
import { Lede, PAGE, TOUCH } from "../app/phone-ui";
import { useMobileShell } from "../app/shell";
import { PhoneMic } from "./PhoneMic";
import { RecentResults } from "./RecentResults";

/** What pairing a computer brings, each with its mark. */
const POINTS: readonly { id: "e2ee" | "safety" | "identity"; icon: IconName }[] = [
  { id: "e2ee", icon: "lock" },
  { id: "safety", icon: "shield" },
  { id: "identity", icon: "alert" },
];

/** The first screen while no computer is paired: talk on the phone right away (docs/dictation.md
 *  §20.7), then what pairing a computer adds. A working screen, not a landing page: the title bar
 *  carries the name, the talk card comes first. */
export function Welcome() {
  const shell = useMobileShell();
  const t = useT();
  return (
    <div className={PAGE}>
      <Lede>{t("mobile.welcome.intro")}</Lede>
      <PhoneMic desktops={[]} />
      <RecentResults />
      <section aria-labelledby="welcome-pairing">
        <Card className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <h2 id="welcome-pairing" className="eyebrow">
              {t("mobile.welcome.pairing")}
            </h2>
            <p className="text-[13px] leading-5 text-fg-muted">{t("mobile.welcome.pairingBody")}</p>
          </div>
          <ul className="flex flex-col gap-3">
            {POINTS.map((point) => (
              <li key={point.id} className="flex gap-3">
                <Icon name={point.icon} size={16} className="mt-0.5 shrink-0 text-fg-muted" />
                <div className="min-w-0">
                  <div className="text-[14px] font-medium text-fg">
                    {t(`mobile.welcome.${point.id}`)}
                  </div>
                  <div className="text-[12px] leading-5 text-fg-muted">
                    {t(`mobile.welcome.${point.id}Body`)}
                  </div>
                </div>
              </li>
            ))}
          </ul>
          <Button
            variant="primary"
            icon="monitor"
            className={`${TOUCH} w-full`}
            onClick={() => {
              shell.go("device");
            }}>
            {t("mobile.welcome.start")}
          </Button>
        </Card>
      </section>
    </div>
  );
}
