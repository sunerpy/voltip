import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { CodeInput } from "./CodeInput";
import { SafetyCodeView } from "./SafetyCodeView";

function Harness({ onComplete }: { onComplete: (d: string) => void }) {
  const [value, setValue] = useState("");
  return <CodeInput value={value} onChange={setValue} onComplete={onComplete} autoFocus />;
}

describe("CodeInput", () => {
  it("shows six cells split 3-3, accepts digits only and fires onComplete once", async () => {
    const user = userEvent.setup();
    const onComplete = vi.fn();
    const { container } = render(<Harness onComplete={onComplete} />);
    expect(container.querySelectorAll("[data-cell]")).toHaveLength(6);
    expect(screen.getAllByText("_")).toHaveLength(6);
    const input = screen.getByLabelText("六位配对码");
    expect(input).toHaveFocus();
    await user.type(input, "48a3");
    expect(container.querySelectorAll('[data-filled="true"]')).toHaveLength(3);
    expect(onComplete).not.toHaveBeenCalled();
    await user.type(input, "921");
    expect(onComplete).toHaveBeenCalledWith("483921");
    expect(onComplete).toHaveBeenCalledTimes(1);
    await user.click(container.querySelector("[data-cell='0']") ?? container);
    expect(input).toHaveFocus();
  });

  it("renders error and disabled states", () => {
    render(<CodeInput value="12" onChange={vi.fn()} error="验证码不正确" disabled />);
    expect(screen.getByRole("alert")).toHaveTextContent("验证码不正确");
    expect(screen.getByLabelText("六位配对码")).toBeDisabled();
  });
});

describe("SafetyCodeView", () => {
  it("lists four words and the fingerprint", () => {
    render(
      <SafetyCodeView
        code={{ words: ["amber", "boat", "cedar", "delta"], fingerprint: "A7:C4 · 3D:F2" }}
        size="sm"
      />,
    );
    expect(screen.getByRole("list", { name: "安全码" }).children).toHaveLength(4);
    expect(screen.getByLabelText("指纹")).toHaveTextContent("A7:C4 · 3D:F2");
  });
});
