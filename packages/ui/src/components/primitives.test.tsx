import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { cx } from "../cx";
import { Badge } from "./Badge";
import { Banner } from "./Banner";
import { Button } from "./Button";
import { Card } from "./Card";
import { Chip } from "./Chip";
import { EmptyState } from "./EmptyState";
import { Eyebrow } from "./Eyebrow";
import { ICON_NAMES, Icon, isIconName } from "./Icon";
import { IconButton } from "./IconButton";
import { Input, Textarea } from "./Input";
import { Keycap, Keycaps, splitKeys } from "./Keycap";
import { Lamp } from "./Lamp";
import { LampText } from "./LampText";
import { Panel } from "./Panel";
import { Readout } from "./Readout";
import { Segmented } from "./Segmented";
import { Select } from "./Select";
import { StatusRow } from "./StatusRow";
import { Toggle } from "./Toggle";

describe("cx", () => {
  it("joins truthy strings only", () => {
    expect(cx("a", false, null, undefined, "", "b")).toBe("a b");
  });
});

describe("Icon", () => {
  it("renders every named path and exposes the name list", () => {
    for (const name of ICON_NAMES) {
      const { unmount } = render(<Icon name={name} />);
      expect(document.querySelector(`[data-icon="${name}"]`)).not.toBeNull();
      unmount();
    }
    expect(isIconName("home")).toBe(true);
    expect(isIconName("nope")).toBe(false);
    render(<Icon name="home" aria-label="首页" />);
    expect(screen.getByLabelText("首页")).not.toHaveAttribute("aria-hidden");
  });
});

describe("Lamp / LampText / Badge / Chip", () => {
  it("maps tones to token classes and renders labels", () => {
    const { container } = render(
      <>
        <Lamp tone="ok" label="就绪" />
        <Lamp tone="idle" pulse size={10} />
        <Lamp tone="off" size={6} />
        <Lamp tone="danger" />
        <Lamp tone="warn" />
        <Lamp tone="accent" />
        <Lamp tone="neutral" />
      </>,
    );
    expect(screen.getByRole("img", { name: "就绪" })).toHaveClass("bg-ok");
    expect(container.querySelector('[data-tone="idle"]')).toHaveClass("border-fg-subtle");
    expect(container.querySelector('[data-tone="off"]')).toHaveClass("bg-border");
    render(
      <LampText tone="ok" readout="127.0.0.1:47823" mono pulse size="sm">
        运行中
      </LampText>,
    );
    expect(screen.getByText("运行中")).toBeInTheDocument();
    expect(screen.getByText("127.0.0.1:47823")).toHaveClass("mono");
  });

  it("badge ok = neutral surface + green dot, ink = primary fill", () => {
    const { container } = render(
      <>
        <Badge tone="ok">已配对</Badge>
        <Badge tone="ink" mono>
          ink
        </Badge>
        <Badge tone="danger">缺失</Badge>
        <Badge>默认</Badge>
      </>,
    );
    const ok = container.querySelector('[data-tone="ok"]');
    expect(ok?.querySelector('[data-tone="ok"].bg-ok')).not.toBeNull();
    expect(ok).not.toHaveClass("bg-ok");
    expect(container.querySelector('[data-tone="ink"]')).toHaveClass("bg-primary", "mono");
    expect(screen.getByText("默认")).toHaveClass("bg-inset");
  });

  it("chip is a button when clickable and a span otherwise", async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(
      <>
        <Chip lamp="ok" count="×14" onClick={onClick} active title="tip">
          台湾大学
        </Chip>
        <Chip round disabled onClick={onClick}>
          禁用
        </Chip>
        <Chip>静态</Chip>
      </>,
    );
    await user.click(screen.getByRole("button", { name: /台湾大学/ }));
    expect(onClick).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: /台湾大学/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByText("×14")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "禁用" })).toBeDisabled();
    expect(screen.getByText("静态").closest("span")).not.toBeNull();
    expect(screen.queryByRole("button", { name: "静态" })).toBeNull();
  });
});

describe("Button / IconButton", () => {
  it("renders variants, sizes, icon, keys and loading state", async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(
      <>
        <Button variant="primary" icon="plus" keys="Ctrl N" onClick={onClick}>
          新规则
        </Button>
        <Button variant="ghost" size="sm">
          ghost
        </Button>
        <Button variant="danger">danger</Button>
        <Button variant="text">text</Button>
        <Button loading>loading</Button>
        <IconButton icon="copy" label="复制" size={28} tone="danger" bordered onClick={onClick} />
      </>,
    );
    await user.click(screen.getByRole("button", { name: /新规则/ }));
    await user.click(screen.getByRole("button", { name: "复制" }));
    expect(onClick).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("button", { name: /新规则/ })).toHaveAttribute(
      "data-variant",
      "primary",
    );
    expect(screen.getByText("Ctrl")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "loading" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "loading" })).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("button", { name: "ghost" })).toHaveClass("h-7");
    expect(screen.getByRole("button", { name: "danger" })).toHaveClass("bg-danger");
    expect(screen.getByRole("button", { name: "text" })).toHaveClass("text-accent-text");
  });
});

describe("Toggle / Segmented / Select", () => {
  it("toggle flips via click and respects disabled", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <>
        <Toggle checked={false} onChange={onChange} label="接受手机连接" readout="PORT 8756" />
        <Toggle checked onChange={onChange} disabled id="t2" />
      </>,
    );
    await user.click(screen.getByRole("switch", { name: /接受手机连接/ }));
    expect(onChange).toHaveBeenCalledWith(true);
    expect(screen.getByText("PORT 8756")).toBeInTheDocument();
    const disabled = screen.getAllByRole("switch")[1];
    expect(disabled).toBeDisabled();
    expect(disabled).toHaveAttribute("aria-checked", "true");
  });

  it("segmented selects and blocks disabled segments with a reason", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <Segmented
        label="模式"
        variant="ink"
        size="sm"
        mono
        value="local"
        onChange={onChange}
        options={[
          { value: "local", label: "本地" },
          { value: "cloud", label: "云端" },
          { value: "off", label: "关", disabled: true, reason: "不可用" },
        ]}
      />,
    );
    expect(screen.getByRole("radio", { name: "本地" })).toHaveAttribute("aria-checked", "true");
    await user.click(screen.getByRole("radio", { name: "云端" }));
    expect(onChange).toHaveBeenCalledWith("cloud");
    expect(screen.getByRole("radio", { name: "关" })).toBeDisabled();
    expect(screen.getByRole("radio", { name: "关" })).toHaveAttribute("title", "不可用");
    render(<Segmented value="a" onChange={onChange} options={[{ value: "a", label: "A" }]} />);
    expect(screen.getByRole("radio", { name: "A" })).toHaveClass("bg-surface");
  });

  it("select emits typed values", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <Select
        label="保留时长"
        mono
        size="sm"
        value="30"
        onChange={onChange}
        options={[
          { value: "7", label: "7 天" },
          { value: "30", label: "30 天" },
        ]}
      />,
    );
    await user.selectOptions(screen.getByLabelText("保留时长"), "7");
    expect(onChange).toHaveBeenCalledWith("7");
  });
});

describe("Input / Textarea / Keycap", () => {
  it("renders icon, keycaps, error and help", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <>
        <Input
          label="搜索"
          icon="search"
          keys="Ctrl F"
          mono
          placeholder="文本、应用或引擎"
          onChange={onChange}
          size="sm"
        />
        <Input label="端口" error="端口被占用" size="lg" />
        <Input label="名称" help="≤ 64 字符" />
        <Textarea label="识别原文" mono defaultValue="voltip" />
      </>,
    );
    await user.type(screen.getByLabelText("搜索"), "la");
    expect(onChange).toHaveBeenCalled();
    expect(screen.getByText("Ctrl")).toBeInTheDocument();
    expect(screen.getByLabelText("端口")).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText("端口被占用")).toBeInTheDocument();
    expect(screen.getByText("≤ 64 字符")).toBeInTheDocument();
    expect(screen.getByLabelText("识别原文")).toHaveValue("voltip");
  });

  it("splits key strings on + and whitespace", () => {
    expect(splitKeys("Ctrl+Alt+Space")).toEqual(["Ctrl", "Alt", "Space"]);
    expect(splitKeys("Ctrl  K")).toEqual(["Ctrl", "K"]);
    render(<Keycaps keys="Ctrl+Alt+Space" plus />);
    expect(screen.getAllByText("+")).toHaveLength(2);
    render(<Keycap>Esc</Keycap>);
    expect(screen.getByText("Esc").tagName).toBe("KBD");
  });
});

describe("Card / Panel / Eyebrow / Readout / StatusRow / EmptyState / Banner", () => {
  // Regression (2026-09-27 smoke screenshots): a long right side squeezed the eyebrow onto two
  // lines ("MICROPHONE / INPUT"); now the right side wraps below and the eyebrow stays whole.
  it("keeps a panel's eyebrow on one line and lets the right side wrap", () => {
    render(
      <Panel
        eyebrow="MICROPHONE INPUT"
        right={<span>Monitoring level · waiting for the hotkey</span>}>
        body
      </Panel>,
    );
    const eyebrow = screen.getByText("MICROPHONE INPUT");
    expect(eyebrow.className).toContain("whitespace-nowrap");
    const header = eyebrow.closest("header");
    expect(header?.className).toContain("flex-wrap");
    expect(
      screen.getByText("Monitoring level · waiting for the hotkey").parentElement?.className,
    ).toContain("flex-wrap");
  });

  it("renders containers with their slots", () => {
    render(
      <>
        <Card padding="none" radius={14} interactive selected data-testid="card">
          body
        </Card>
        <Card padding="sm">sm</Card>
        <Panel eyebrow="PAIRING" title="配对新设备" right={<span>WINDOW OPEN</span>}>
          panel body
        </Panel>
        <Eyebrow right="12 s">SYNC</Eyebrow>
        <Readout label="UTTERANCES" value="23" unit="条" size="lg" align="right" />
        <Readout label="LAT" value="—" muted size="sm" />
        <StatusRow label="跟随系统" help="开启后跟随操作系统" note="prefers-color-scheme: light">
          <span>ctl</span>
        </StatusRow>
        <EmptyState
          title="历史记录已关闭"
          mono="history.sqlite3 · 未创建"
          icon="history"
          actions={<button>开启</button>}>
          什么都没有被记录
        </EmptyState>
        <EmptyState title="紧凑" compact />
      </>,
    );
    expect(screen.getByTestId("card")).toHaveClass(
      "rounded-14",
      "inset-ring-2",
      "inset-ring-primary",
    );
    expect(screen.getByText("PAIRING")).toBeInTheDocument();
    expect(screen.getByText("配对新设备")).toBeInTheDocument();
    expect(screen.getByText("WINDOW OPEN")).toBeInTheDocument();
    expect(screen.getByText("SYNC")).toHaveClass("eyebrow");
    expect(screen.getByText("12 s")).toBeInTheDocument();
    expect(screen.getByText("23")).toHaveClass("text-[18px]");
    expect(screen.getByText("—")).toHaveClass("text-fg-subtle");
    expect(screen.getByText("prefers-color-scheme: light")).toBeInTheDocument();
    expect(screen.getByText("历史记录已关闭")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "开启" })).toBeInTheDocument();
    expect(screen.getByText("紧凑")).toHaveClass("text-[14px]");
  });

  it("banner tones, markers and dismiss", async () => {
    const user = userEvent.setup();
    const onDismiss = vi.fn();
    render(
      <>
        <Banner
          tone="danger"
          marker="bar"
          title="载入失败"
          actions={<button>重试</button>}
          onDismiss={onDismiss}>
          详情
        </Banner>
        <Banner tone="warn" marker="icon">
          警告
        </Banner>
        <Banner tone="info" marker="bar">
          info
        </Banner>
        <Banner tone="ok">ok</Banner>
        <Banner>neutral</Banner>
      </>,
    );
    expect(screen.getByRole("alert")).toHaveClass("border-l-danger", "bg-danger-soft");
    await user.click(screen.getByRole("button", { name: "关闭" }));
    expect(onDismiss).toHaveBeenCalled();
    expect(screen.getAllByRole("status")).toHaveLength(4);
  });
});
