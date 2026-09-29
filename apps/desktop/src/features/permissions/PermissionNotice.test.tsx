import { type PermissionReport, translate } from "@voltip/shared";
import { MockBackend, desktopIdentity, sampleDevices } from "@voltip/shared/mock";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";

const mac = () => ({ ...desktopIdentity(), platform: "macos" as const, name: "MacBook Pro" });

const macReport = (overrides: Partial<PermissionReport> = {}): PermissionReport => ({
  platform: "macos",
  microphone: "granted",
  accessibility: "granted",
  ...overrides,
});

describe("the home page's permission notice", () => {
  it("regression: without the forced guide, macOS asks for Accessibility on the home page and the notice leaves once granted", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      mock: { identity: mac(), permissions: macReport({ accessibility: "not_determined" }) },
    });
    expect(await screen.findByTestId("permission-notice")).toHaveTextContent(
      "Voltip 需要「辅助功能」权限，才能把识别结果写入其他应用",
    );
    expect(screen.getByText("还差一个系统权限")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "去授权" }));
    expect(backend.permissionRequests).toEqual(["accessibility"]);
    await waitFor(() => {
      expect(screen.queryByTestId("permission-notice")).not.toBeInTheDocument();
    });
  });

  it("regression: the Accessibility notice says the grant works at once, without a restart", async () => {
    // User feedback 2026-09-29: the notice said to restart Voltip after granting Accessibility,
    // yet it worked without one: the paste and the focused-field lookup ask the system on every
    // use, the notice leaves on the next poll, and a lone-key trigger that failed for the
    // permission is watched again once it is granted.
    renderApp({ mock: { identity: mac(), permissions: macReport({ accessibility: "denied" }) } });
    const notice = await screen.findByTestId("permission-notice");
    expect(notice).toHaveTextContent("开启后立即生效，无需重启 Voltip");
    expect(notice).not.toHaveTextContent("授予后需重启");
    const en = translate("en", "home.permission.accessibility");
    expect(en).toContain("no need to restart Voltip");
    expect(en).not.toMatch(/restart Voltip after/);
    // The setup guide's permission table says the same.
    for (const locale of ["zh-CN", "en"] as const) {
      const purpose = translate(locale, "onboarding.permission.row.accessibility.purpose");
      expect(purpose).not.toMatch(/重启|restart/);
    }
  });

  it("names a denied microphone first and opens the setup guide on request", async () => {
    const user = userEvent.setup();
    renderApp({
      mock: {
        identity: mac(),
        permissions: macReport({ microphone: "denied", accessibility: "denied" }),
      },
    });
    expect(await screen.findByTestId("permission-notice")).toHaveTextContent("麦克风权限已被拒绝");
    await user.click(screen.getByRole("button", { name: "打开设置向导" }));
    expect(await screen.findByRole("heading", { name: "系统权限", level: 2 })).toBeInTheDocument();
  });

  it("stays away where nothing blocks: granted macOS, Windows, Linux", async () => {
    for (const permissions of [
      macReport(),
      { platform: "windows", microphone: "granted", accessibility: "not_applicable" },
      { platform: "linux", microphone: "not_applicable", accessibility: "not_applicable" },
    ] satisfies PermissionReport[]) {
      const backend = new MockBackend({
        devices: sampleDevices(1_758_700_000),
        now: () => 1_758_700_000_000,
        permissions,
      });
      const reads = vi.spyOn(backend, "permissionsStatus");
      const { unmount } = renderApp({ backend });
      expect(await screen.findByTestId("page-home")).toBeInTheDocument();
      await waitFor(() => {
        expect(reads).toHaveBeenCalled();
      });
      expect(screen.queryByTestId("permission-notice")).not.toBeInTheDocument();
      unmount();
    }
  });
});
