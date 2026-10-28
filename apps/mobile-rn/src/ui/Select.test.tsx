// The pickers on their own (docs/mobile-rn.md §5): a field, a chip and a settings row that open the
// choices in a bottom sheet; the screens' tests cover them in place.
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react-native";
import { type ReactNode, useState } from "react";
import { DeviceEventEmitter } from "react-native";
import { Provider as PaperProvider } from "react-native-paper";
import { SafeAreaProvider } from "react-native-safe-area-context";

import { METRICS } from "../test/render";
import { appTheme } from "../theme/themes";
import { type SelectOption, SelectChip, SelectField, SelectRow } from "./Select";

type Fruit = "apple" | "pear" | "plum" | "fig";

const OPTIONS: SelectOption<Fruit>[] = [
  { value: "apple", label: "苹果", group: "常见" },
  { value: "pear", label: "梨", description: "秋天的", group: "常见" },
  { value: "plum", label: "李子", disabled: true, group: "少见" },
  { value: "fig", label: "无花果", group: "少见" },
];

function Frame({ children }: { children: ReactNode }) {
  return (
    <SafeAreaProvider initialMetrics={METRICS}>
      <PaperProvider theme={appTheme("light")}>{children}</PaperProvider>
    </SafeAreaProvider>
  );
}

/** A picker that keeps its value, as a page would. */
function Held({
  kind,
  initial,
  options = OPTIONS,
  onChange,
  onOpen,
  disabled,
  testID,
  icon,
}: {
  kind: "field" | "chip" | "row";
  initial: Fruit;
  options?: SelectOption<Fruit>[];
  onChange?: (v: Fruit) => void;
  onOpen?: () => void;
  disabled?: boolean;
  testID?: string;
  icon?: string;
}) {
  const [value, setValue] = useState<Fruit>(initial);
  const props = {
    label: "水果",
    value,
    options,
    onChange: (v: Fruit) => {
      setValue(v);
      onChange?.(v);
    },
    ...(onOpen === undefined ? {} : { onOpen }),
    ...(disabled === undefined ? {} : { disabled }),
    ...(testID === undefined ? {} : { testID }),
  };
  if (kind === "field") return <SelectField {...props} />;
  if (kind === "chip") return <SelectChip {...props} {...(icon === undefined ? {} : { icon })} />;
  return <SelectRow {...props} {...(icon === undefined ? {} : { icon })} />;
}

async function closed(testID: string) {
  await waitFor(() => {
    expect(screen.queryByTestId(`${testID}-sheet`)).toBeNull();
  });
}

describe("SelectField", () => {
  it("opens the choices in groups, the current one checked; a new choice is kept, the same one asks nothing", async () => {
    const onChange = jest.fn();
    const onOpen = jest.fn();
    await render(
      <Frame>
        <Held kind="field" initial="pear" onChange={onChange} onOpen={onOpen} testID="fruit" />
      </Frame>,
    );
    const field = screen.getByTestId("fruit");
    expect(field).toHaveAccessibleName("水果");
    expect(within(field).getByText("梨")).toBeOnTheScreen();
    await fireEvent.press(field);
    expect(onOpen).toHaveBeenCalledTimes(1);
    const sheet = within(await screen.findByTestId("fruit-sheet"));
    expect(sheet.getByRole("header", { name: "水果" })).toBeOnTheScreen();
    expect(sheet.getByRole("header", { name: "常见" })).toBeOnTheScreen();
    expect(sheet.getByRole("header", { name: "少见" })).toBeOnTheScreen();
    expect(sheet.getByText("秋天的")).toBeOnTheScreen();
    expect(screen.getByTestId("fruit-option-pear")).toBeChecked();
    expect(screen.getByTestId("fruit-option-plum")).toBeDisabled();
    await fireEvent.press(screen.getByTestId("fruit-option-pear"));
    expect(onChange).not.toHaveBeenCalled();
    await closed("fruit");
    await fireEvent.press(field);
    await fireEvent.press(await screen.findByTestId("fruit-option-fig"));
    expect(onChange).toHaveBeenCalledWith("fig");
    await closed("fruit");
    expect(within(field).getByText("无花果")).toBeOnTheScreen();
  });

  it("stands in the first choice for a value no option has, and choosing it keeps it", async () => {
    const onChange = jest.fn();
    const options = OPTIONS.filter((o) => o.value !== "pear");
    await render(
      <Frame>
        <Held kind="field" initial="pear" options={options} onChange={onChange} testID="fruit" />
      </Frame>,
    );
    expect(within(screen.getByTestId("fruit")).getByText("苹果")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("fruit"));
    await fireEvent.press(await screen.findByTestId("fruit-option-apple"));
    expect(onChange).toHaveBeenCalledWith("apple");
  });

  it("shows nothing for an empty list, and a disabled field does not open", async () => {
    const onOpen = jest.fn();
    await render(
      <Frame>
        <Held kind="field" initial="pear" options={[]} onOpen={onOpen} disabled testID="fruit" />
      </Frame>,
    );
    expect(screen.getByTestId("fruit")).toBeDisabled();
    await fireEvent.press(screen.getByTestId("fruit"));
    expect(onOpen).not.toHaveBeenCalled();
    expect(screen.queryByTestId("fruit-sheet")).toBeNull();
  });

  it("checks a value listed in two groups where it first appears; the scrim and Android's back close the sheet", async () => {
    const twice: SelectOption<Fruit>[] = [
      { value: "fig", label: "无花果", group: "最近" },
      ...OPTIONS,
    ];
    await render(
      <Frame>
        <Held kind="field" initial="fig" options={twice} testID="fruit" />
      </Frame>,
    );
    await fireEvent.press(screen.getByTestId("fruit"));
    expect(await screen.findByTestId("fruit-option-fig")).toBeChecked();
    expect(screen.getByTestId("fruit-again-option-fig")).not.toBeChecked();
    // The scrim is hidden from screen readers (the sheet is modal); a finger can still tap it.
    await fireEvent.press(screen.getByTestId("fruit-scrim", { includeHiddenElements: true }));
    await closed("fruit");
    await fireEvent.press(screen.getByTestId("fruit"));
    expect(await screen.findByTestId("fruit-sheet")).toBeOnTheScreen();
    await act(async () => {
      DeviceEventEmitter.emit("hardwareBackPress");
      await Promise.resolve();
    });
    await closed("fruit");
  });

  it("works without test ids: the options are found by their names", async () => {
    const onChange = jest.fn();
    await render(
      <Frame>
        <Held kind="field" initial="apple" onChange={onChange} />
      </Frame>,
    );
    await fireEvent.press(screen.getByLabelText("水果"));
    await fireEvent.press(await screen.findByRole("radio", { name: "梨" }));
    expect(onChange).toHaveBeenCalledWith("pear");
  });
});

describe("SelectChip", () => {
  it("names the picker and its choice, with an icon, and opens the same sheet", async () => {
    const onChange = jest.fn();
    await render(
      <Frame>
        <Held kind="chip" initial="apple" icon="fruit-cherries" onChange={onChange} testID="chip" />
      </Frame>,
    );
    expect(screen.getByTestId("chip")).toHaveTextContent(/水果 · 苹果/);
    await fireEvent.press(screen.getByTestId("chip"));
    await fireEvent.press(await screen.findByTestId("chip-option-fig"));
    expect(onChange).toHaveBeenCalledWith("fig");
    expect(screen.getByTestId("chip")).toHaveTextContent(/水果 · 无花果/);
  });

  it("without a choice, an icon or a test id it still names the picker; disabled, it does not open", async () => {
    await render(
      <Frame>
        <Held kind="chip" initial="apple" options={[]} disabled />
      </Frame>,
    );
    const chip = screen.getByText("水果 · ");
    await fireEvent.press(chip);
    expect(screen.queryByRole("header", { name: "水果" })).toBeNull();
  });
});

describe("SelectRow", () => {
  it("is a settings row: the title over the choice, with or without an icon", async () => {
    const onChange = jest.fn();
    await render(
      <Frame>
        <Held kind="row" initial="pear" icon="fruit-pear" onChange={onChange} testID="row" />
        <Held kind="row" initial="apple" />
      </Frame>,
    );
    expect(screen.getByTestId("row")).toHaveTextContent(/水果梨/);
    await fireEvent.press(screen.getByTestId("row"));
    await fireEvent.press(await screen.findByTestId("row-option-apple"));
    expect(onChange).toHaveBeenCalledWith("apple");
    await closed("row");
    expect(screen.getAllByText("苹果")).toHaveLength(2);
  });
});
