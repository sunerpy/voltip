import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { type CellSpec, Table, type TableColumn, renderCell } from "./Table";

interface Row {
  id: string;
  name: string;
  online: boolean;
}

const rows: Row[] = [
  { id: "a", name: "Pixel 8", online: true },
  { id: "b", name: "iPhone", online: false },
];

describe("Table", () => {
  it("renders columns, selects rows via click and keyboard, expands rows and shows empty state", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const columns: TableColumn<Row>[] = [
      {
        id: "name",
        header: "设备",
        cell: (r) => ({ type: "two", primary: r.name, secondary: r.id }),
      },
      {
        id: "state",
        header: "状态",
        align: "right",
        mono: false,
        cell: (r) => ({
          type: "lamp",
          tone: r.online ? "ok" : "idle",
          text: r.online ? "在线" : "离线",
          mono: true,
        }),
      },
      { id: "raw", header: "raw", align: "center", cell: (r) => <em>{r.id}</em> },
    ];
    render(
      <Table
        label="设备表"
        columns={columns}
        rows={rows}
        rowKey={(r) => r.id}
        selectedKey="a"
        onSelect={onSelect}
        rowClassName={(r) => (r.online ? undefined : "opacity-50")}
        expandRow={(r) => (r.id === "a" ? <div>expanded</div> : null)}
        dense
      />,
    );
    const table = screen.getByRole("table", { name: "设备表" });
    expect(within(table).getByText("Pixel 8")).toBeInTheDocument();
    expect(within(table).getByText("expanded")).toBeInTheDocument();
    const rowB = within(table).getByText("iPhone").closest("tr");
    if (!rowB) throw new Error("row");
    await user.click(rowB);
    expect(onSelect).toHaveBeenCalledWith(rows[1]);
    rowB.focus();
    await user.keyboard("{Enter}");
    await user.keyboard(" ");
    expect(onSelect).toHaveBeenCalledTimes(3);
    expect(rowB).toHaveClass("opacity-50");
    expect(within(table).getByText("Pixel 8").closest("tr")).toHaveAttribute(
      "data-selected",
      "true",
    );

    render(<Table columns={columns} rows={[]} rowKey={(r) => r.id} empty={<div>空</div>} />);
    expect(screen.getByText("空")).toBeInTheDocument();
  });

  it("renders every cell spec type and stops propagation on embedded controls", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onToggle = vi.fn();
    const onLink = vi.fn();
    const onRadio = vi.fn();
    const onAction = vi.fn();
    const specs: CellSpec[] = [
      { type: "text", text: "t", muted: true, strike: true },
      { type: "mono", text: "m", muted: true },
      { type: "two", primary: "p", secondary: "s", strikeSecondary: true },
      { type: "chip", text: "chip", lamp: "ok" },
      { type: "badge", text: "badge", tone: "ok", mono: true },
      { type: "lamp", tone: "danger", text: "lamp", pulse: true },
      { type: "toggle", checked: false, onChange: onToggle, label: "启用" },
      { type: "progress", value: 0.4, text: "40%" },
      { type: "keys", keys: "Ctrl C" },
      { type: "link", text: "link", onClick: onLink },
      { type: "radio", checked: false, onChange: onRadio, label: "选择", reason: "r" },
      {
        type: "actions",
        actions: [
          { label: "批准", onClick: onAction, tone: "primary" },
          { label: "吊销", onClick: onAction, tone: "danger" },
          { label: "编辑", onClick: onAction, tone: "default" },
          { label: "隐藏", onClick: onAction, hidden: true },
        ],
      },
    ];
    const columns: TableColumn<CellSpec>[] = [{ id: "c", header: "cell", cell: (s) => s }];
    render(<Table columns={columns} rows={specs} rowKey={(s) => s.type} onSelect={onSelect} />);
    await user.click(screen.getByRole("switch"));
    expect(onToggle).toHaveBeenCalledWith(true);
    await user.click(screen.getByRole("button", { name: "link" }));
    expect(onLink).toHaveBeenCalled();
    await user.click(screen.getByRole("radio", { name: "选择" }));
    expect(onRadio).toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "批准" }));
    await user.click(screen.getByRole("button", { name: "吊销" }));
    await user.click(screen.getByRole("button", { name: "编辑" }));
    expect(onAction).toHaveBeenCalledTimes(3);
    expect(screen.queryByRole("button", { name: "隐藏" })).toBeNull();
    expect(onSelect).not.toHaveBeenCalled();
    expect(screen.getByText("t")).toHaveClass("line-through");
    expect(screen.getByText("s")).toHaveClass("line-through");
    expect(screen.getByText("40%")).toBeInTheDocument();
    expect(renderCell({ type: "text", text: "x" })).toBeTruthy();
  });

  it("regression: cells are single truncated lines; flexible columns share width, fixed ones are capped", () => {
    const columns: TableColumn<Row>[] = [
      { id: "name", header: "名称", minWidth: 120, cell: (r) => ({ type: "text", text: r.name }) },
      { id: "note", header: "备注", cell: (r) => ({ type: "text", text: r.id }) },
      {
        id: "state",
        header: "状态",
        width: 84,
        cell: (r) => ({ type: "text", text: r.online ? "在线" : "离线" }),
      },
    ];
    render(<Table label="t" columns={columns} rows={rows} rowKey={(r) => r.id} />);
    // jsdom's computed style drops min/max-width, so read the inline declaration React wrote.
    const style = (el: HTMLElement | undefined) => el?.getAttribute("style") ?? "";
    const headers = screen.getAllByRole("columnheader");
    expect(style(headers[0])).toBe("width: 50%; max-width: 0; min-width: 120px;");
    expect(style(headers[1])).toBe("width: 50%; max-width: 0;");
    expect(style(headers[2])).toBe("width: 84px; max-width: 84px;");
    const firstRow = screen.getAllByRole("row")[1];
    if (firstRow === undefined) throw new Error("no body row");
    const firstCells = within(firstRow).getAllByRole("cell");
    for (const cell of firstCells) {
      expect(cell.className).toContain("whitespace-nowrap");
      expect(cell.className).toContain("text-ellipsis");
      expect(cell.className).toContain("overflow-hidden");
    }
    expect(style(firstCells[2])).toBe("width: 84px; max-width: 84px;");
  });
});
