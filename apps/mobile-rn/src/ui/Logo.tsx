// The app mark (packages/ui Logo): a navy rounded square with a V whose left arm is pale and right
// arm orange. The icon's own colours, the same in every theme.
import Svg, { Polygon, Rect } from "react-native-svg";

export function Logo({ size = 24 }: { size?: number }) {
  return (
    <Svg width={size} height={size} viewBox="0 0 100 100" accessible={false}>
      <Rect width="100" height="100" rx="22" fill="#0B1220" />
      <Polygon points="22.5,22 39.5,22 50,46 50,79" fill="#E7EDF5" />
      <Polygon points="60.5,22 77.5,22 50,79 50,46" fill="#F97316" />
    </Svg>
  );
}
