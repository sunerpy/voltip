import { type ReactNode, isValidElement } from "react";
import { cx } from "../cx";
import { Badge, type BadgeTone } from "./Badge";
import { Button } from "./Button";
import { Chip } from "./Chip";
import { Keycaps } from "./Keycap";
import { Lamp, type LampTone } from "./Lamp";
import { Progress } from "./Progress";
import { Toggle } from "./Toggle";

export type CellSpec =
  | { type: "text"; text: ReactNode; muted?: boolean; strike?: boolean }
  | { type: "mono"; text: ReactNode; muted?: boolean }
  | { type: "two"; primary: ReactNode; secondary: ReactNode; strikeSecondary?: boolean }
  | { type: "chip"; text: string; lamp?: LampTone }
  | { type: "badge"; text: string; tone: BadgeTone; mono?: boolean }
  | { type: "lamp"; tone: LampTone; text: ReactNode; mono?: boolean; pulse?: boolean }
  | {
      type: "toggle";
      checked: boolean;
      onChange: (next: boolean) => void;
      disabled?: boolean;
      label: string;
    }
  | { type: "progress"; value: number; text?: ReactNode }
  | { type: "keys"; keys: string }
  | { type: "link"; text: ReactNode; onClick: () => void }
  | {
      type: "radio";
      checked: boolean;
      onChange: () => void;
      disabled?: boolean;
      label: string;
      reason?: string;
    }
  | {
      type: "actions";
      actions: {
        label: string;
        onClick: () => void;
        tone?: "default" | "danger" | "primary";
        hidden?: boolean;
      }[];
    };

export interface TableColumn<Row> {
  id: string;
  header: ReactNode;
  width?: number | string;
  /** Lower bound for a flexible (no `width`) column, so truncation never eats the whole cell. */
  minWidth?: number;
  align?: "left" | "right" | "center";
  mono?: boolean;
  cell: (row: Row, index: number) => CellSpec | ReactNode;
}

export interface TableProps<Row> {
  columns: readonly TableColumn<Row>[];
  rows: readonly Row[];
  rowKey: (row: Row) => string;
  selectedKey?: string;
  onSelect?: (row: Row) => void;
  /** Rendered in place of the body when `rows` is empty. */
  empty?: ReactNode;
  /** Extra classes per row (e.g. dim revoked rows). */
  rowClassName?: (row: Row) => string | undefined;
  /** Renders a full-width row right after the given row (download progress). */
  expandRow?: (row: Row) => ReactNode;
  dense?: boolean;
  label?: string;
  className?: string;
}

function isSpec(value: CellSpec | ReactNode): value is CellSpec {
  return (
    typeof value === "object" &&
    value !== null &&
    !isValidElement(value) &&
    "type" in value &&
    typeof value.type === "string"
  );
}

export function renderCell(spec: CellSpec): ReactNode {
  switch (spec.type) {
    case "text":
      return (
        <span className={cx(spec.muted && "text-fg-muted", spec.strike && "line-through")}>
          {spec.text}
        </span>
      );
    case "mono":
      return <span className={cx("mono", spec.muted && "text-fg-muted")}>{spec.text}</span>;
    case "two":
      return (
        <span className="flex flex-col leading-tight">
          <span className="truncate text-fg">{spec.primary}</span>
          <span
            className={cx(
              "mono text-[11px] text-fg-subtle",
              spec.strikeSecondary && "line-through",
            )}>
            {spec.secondary}
          </span>
        </span>
      );
    case "chip":
      return <Chip lamp={spec.lamp}>{spec.text}</Chip>;
    case "badge":
      return (
        <Badge tone={spec.tone} mono={spec.mono}>
          {spec.text}
        </Badge>
      );
    case "lamp":
      return (
        <span
          className={cx("inline-flex items-center gap-1.5 whitespace-nowrap", spec.mono && "mono")}>
          <Lamp tone={spec.tone} pulse={spec.pulse} />
          {spec.text}
        </span>
      );
    case "toggle":
      return (
        <span
          onClick={(e) => {
            e.stopPropagation();
          }}>
          <Toggle
            checked={spec.checked}
            onChange={spec.onChange}
            disabled={spec.disabled}
            ariaLabel={spec.label}
          />
        </span>
      );
    case "progress":
      return (
        <span className="flex items-center gap-2">
          <Progress value={spec.value} className="w-20" />
          {spec.text !== undefined && (
            <span className="mono text-[11px] text-fg-muted">{spec.text}</span>
          )}
        </span>
      );
    case "keys":
      return <Keycaps keys={spec.keys} />;
    case "link":
      return (
        <button
          type="button"
          className="text-accent-text hover:text-accent-text-hover"
          onClick={(e) => {
            e.stopPropagation();
            spec.onClick();
          }}>
          {spec.text}
        </button>
      );
    case "radio":
      return (
        <input
          type="radio"
          aria-label={spec.label}
          title={spec.reason}
          checked={spec.checked}
          disabled={spec.disabled}
          onClick={(e) => {
            e.stopPropagation();
          }}
          onChange={spec.onChange}
          className="h-3.5 w-3.5 accent-[var(--primary)]"
        />
      );
    case "actions":
      return (
        <span className="inline-flex items-center justify-end gap-1">
          {spec.actions
            .filter((a) => !a.hidden)
            .map((a) => (
              <Button
                key={a.label}
                size="sm"
                variant={
                  a.tone === "primary"
                    ? "primary"
                    : a.tone === "danger"
                      ? "text-danger"
                      : a.tone === "default"
                        ? "text-muted"
                        : "text"
                }
                onClick={(e) => {
                  e.stopPropagation();
                  a.onClick();
                }}>
                {a.label}
              </Button>
            ))}
        </span>
      );
  }
}

/** Data table with hairline rows; selection via click / Enter / Space when `onSelect` is set. */
export function Table<Row>({
  columns,
  rows,
  rowKey,
  selectedKey,
  onSelect,
  empty,
  rowClassName,
  expandRow,
  dense = false,
  label,
  className,
}: TableProps<Row>) {
  const rowHeight = dense ? "h-7" : "h-[var(--row-h)]";
  // Columns without a fixed width share the leftover space evenly and truncate their cells (the
  // boards render every table as single, truncated lines). `max-width: 0` is what lets a table
  // cell shrink below its content in auto layout; the percentage keeps the share even.
  const flexible = columns.filter((c) => c.width === undefined).length;
  // Fixed columns are capped at their declared width too, so a long value truncates instead of
  // pushing the table past its card.
  const flexStyle = (c: TableColumn<Row>) =>
    c.width === undefined
      ? { width: `${100 / flexible}%`, maxWidth: 0, minWidth: c.minWidth }
      : { width: c.width, maxWidth: c.width };
  return (
    <table aria-label={label} className={cx("w-full border-collapse text-[13px]", className)}>
      <colgroup>
        {columns.map((c) => (
          <col key={c.id} style={c.width === undefined ? undefined : { width: c.width }} />
        ))}
      </colgroup>
      <thead>
        <tr className="border-b border-border">
          {columns.map((c) => (
            <th
              key={c.id}
              scope="col"
              style={flexStyle(c)}
              className={cx(
                "h-7 px-2 text-[11px] font-normal whitespace-nowrap text-fg-subtle",
                c.mono !== false && "mono",
                c.align === "right"
                  ? "text-right"
                  : c.align === "center"
                    ? "text-center"
                    : "text-left",
              )}>
              {c.header}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.length === 0 && empty !== undefined && (
          <tr>
            <td colSpan={columns.length} className="p-0">
              {empty}
            </td>
          </tr>
        )}
        {rows.map((row, index) => {
          const key = rowKey(row);
          const selected = key === selectedKey;
          const expanded = expandRow?.(row);
          return (
            <RowGroup key={key} expanded={expanded} columns={columns.length}>
              <tr
                data-selected={selected ? "true" : undefined}
                tabIndex={onSelect ? 0 : undefined}
                aria-selected={onSelect ? selected : undefined}
                onClick={
                  onSelect
                    ? () => {
                        onSelect(row);
                      }
                    : undefined
                }
                onKeyDown={
                  onSelect
                    ? (e) => {
                        // Only the row's own keys: Enter / Space on a button inside the row
                        // press that button, not the row.
                        if (e.target !== e.currentTarget) return;
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          onSelect(row);
                        }
                      }
                    : undefined
                }
                className={cx(
                  "border-b border-border last:border-b-0",
                  onSelect && "cursor-pointer hover:bg-canvas",
                  selected && "bg-canvas shadow-[inset_2px_0_0_var(--primary)]",
                  rowClassName?.(row),
                )}>
                {columns.map((c) => {
                  const value = c.cell(row, index);
                  return (
                    <td
                      key={c.id}
                      style={flexStyle(c)}
                      className={cx(
                        rowHeight,
                        "overflow-hidden px-2 align-middle text-ellipsis whitespace-nowrap",
                        c.mono && "mono",
                        c.align === "right"
                          ? "text-right"
                          : c.align === "center"
                            ? "text-center"
                            : "text-left",
                      )}>
                      {isSpec(value) ? renderCell(value) : value}
                    </td>
                  );
                })}
              </tr>
            </RowGroup>
          );
        })}
      </tbody>
    </table>
  );
}

function RowGroup({
  children,
  expanded,
  columns,
}: {
  children: ReactNode;
  expanded: ReactNode;
  columns: number;
}) {
  if (expanded === undefined || expanded === null || expanded === false) return <>{children}</>;
  return (
    <>
      {children}
      <tr className="border-b border-border">
        <td colSpan={columns} className="p-0">
          {expanded}
        </td>
      </tr>
    </>
  );
}
