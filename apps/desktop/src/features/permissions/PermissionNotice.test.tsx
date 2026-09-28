import { type PermissionReport } from "@voltip/shared";
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
      "Voltip 需要「辅助功能」权限，才能把识别结果写进其他应用",
    );
    expect(screen.getByText("还差一个系统权限")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "去授权" }));
    expect(backend.permissionRequests).toEqual(["accessibility"]);
    await waitFor(() => {
      expect(screen.queryByTestId("permission-notice")).not.toBeInTheDocument();
    });
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
