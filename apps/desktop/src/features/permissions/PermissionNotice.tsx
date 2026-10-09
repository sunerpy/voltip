import { type PermissionReport, onboardingGate } from "@voltip/shared";
import { Banner, Button, useBackend, useI18n } from "@voltip/ui";
import { useState } from "react";
import { useRouter } from "../../app/router";
import { usePermissions } from "./usePermissions";

/** Home's notice for a permission that stops dictation (docs/dictation.md §15.1): shown only while
 *  `onboardingGate` blocks something — Accessibility on macOS, a denied microphone anywhere. The
 *  first launch no longer opens the setup guide by itself (user feedback 2026-09-28: the defaults
 *  work out of the box), so this is where a missing grant surfaces. It reads once, and keeps
 *  polling every second only while it is shown, so it goes away as soon as the grant arrives. */
export function PermissionNotice() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { navigate } = useRouter();
  // Poll until the first answer, then only while something blocks: the read that shows the grant
  // arrived is the last one.
  const [report, setReport] = useState<PermissionReport | undefined>(undefined);
  const blocking = report === undefined ? [] : onboardingGate(report);
  const permissions = usePermissions(backend, report === undefined || blocking.length > 0);
  if (permissions.report !== report) setReport(permissions.report);
  const first = blocking[0];
  if (first === undefined) return null;
  return (
    <Banner
      tone="warn"
      marker="bar"
      title={t("home.permission.title")}
      actions={
        <>
          <Button
            size="sm"
            variant="primary"
            onClick={() => {
              void permissions.request(first);
            }}>
            {t("home.permission.request")}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              navigate({ name: "onboarding", step: 1 });
            }}>
            {t("home.permission.guide")}
          </Button>
        </>
      }>
      <span data-testid="permission-notice">
        {first === "accessibility"
          ? t("home.permission.accessibility")
          : t("home.permission.microphone")}
      </span>
    </Banner>
  );
}
