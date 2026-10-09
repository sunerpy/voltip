// The kit's building blocks on their own (the screens' tests cover them in place).
import { render, screen } from "@testing-library/react-native";
import { View } from "react-native";
import { Provider as PaperProvider, Text } from "react-native-paper";

import { appTheme } from "../theme/themes";
import { StateLine } from "./kit";

describe("StateLine", () => {
  it("regression: beside a title that takes the rest of a row, it leaves the title its room", async () => {
    // Device Farm, Xiaomi 13, 2026-10-09 (user report 「配对后详情页和很多页面上方空白太多」): the
    // status line's text took the whole row (flex: 1), the computer's name beside it on the
    // device card got no width and wrapped one character per invisible line, and the card grew a
    // screen of blank space at its top; the history entry's header had the same row. The text
    // now takes its own width and shrinks (wrapping) only when the row needs the room.
    await render(
      <PaperProvider theme={appTheme("light")}>
        <View style={{ flexDirection: "row" }}>
          <Text style={{ flex: 1 }}>Device Farm 测试电脑</Text>
          <StateLine tone="ok" testID="line">
            在线 · 经中继
          </StateLine>
        </View>
      </PaperProvider>,
    );
    const text = screen.getByText("在线 · 经中继");
    expect(text).not.toHaveStyle({ flex: 1 });
    expect(text).toHaveStyle({ flexShrink: 1 });
    expect(screen.getByTestId("line")).toHaveStyle({ flexShrink: 1 });
  });
});
