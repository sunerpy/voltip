// The app mark 「声波光标」 (packages/ui Logo): on a deep ink rounded square, three white sound bars
// run into a cyan text cursor. The icon's own colours, the same in every theme. Fitted to the
// physical pixels it covers (`fitLogo` at size × the pixel ratio), so its straight edges are
// sharp; React Native lays views out on whole physical pixels already.
import { fitLogo } from "@voltip/shared";
import { PixelRatio } from "react-native";
import Svg, { Defs, LinearGradient, Rect, Stop } from "react-native-svg";

export function Logo({ size = 24 }: { size?: number }) {
  const pixels = PixelRatio.getPixelSizeForLayoutSize(size);
  const { bars, cursor } = fitLogo(pixels);
  return (
    <Svg width={size} height={size} viewBox={`0 0 ${pixels} ${pixels}`} accessible={false}>
      <Defs>
        <LinearGradient id="tile" x1="0" y1="0" x2="1" y2="1">
          <Stop offset="0" stopColor="#0B1220" />
          <Stop offset="1" stopColor="#1B2A4A" />
        </LinearGradient>
        <LinearGradient id="cursor" x1="0" y1="0" x2="0" y2="1">
          <Stop offset="0" stopColor="#38BDF8" />
          <Stop offset="1" stopColor="#22D3EE" />
        </LinearGradient>
      </Defs>
      <Rect width={pixels} height={pixels} rx={(232 / 1024) * pixels} fill="url(#tile)" />
      {bars.map((bar) => (
        <Rect
          key={bar.x}
          x={bar.x}
          y={bar.y}
          width={bar.width}
          height={bar.height}
          rx={bar.width / 2}
          fill="#FFFFFF"
        />
      ))}
      <Rect
        x={cursor.x}
        y={cursor.y}
        width={cursor.width}
        height={cursor.height}
        rx={cursor.width / 2}
        fill="url(#cursor)"
      />
    </Svg>
  );
}
