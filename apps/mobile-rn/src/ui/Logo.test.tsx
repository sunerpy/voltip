import { render } from "@testing-library/react-native";
import { fitLogo } from "@voltip/shared";
import { PixelRatio } from "react-native";

import { Logo } from "./Logo";

type Node = { type: string; props: Record<string, unknown>; children: Node[] | null };

/** Every host element of the rendered tree, depth first. */
function nodes(tree: unknown): Node[] {
  if (!tree || typeof tree !== "object") return [];
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  const node = tree as Node;
  return [node, ...(node.children ?? []).flatMap(nodes)];
}

describe("Logo", () => {
  afterEach(() => {
    jest.restoreAllMocks();
  });

  it("regression: it is fitted to the physical pixels it covers, its edges on whole pixels", async () => {
    // User 2026-10-08 (清晰度要提高下): a 26 dp mark on a 2.625× phone covers 68 pixels; drawn as
    // designed, its bars' edges fall between them.
    jest.spyOn(PixelRatio, "get").mockReturnValue(2.625);
    const view = await render(<Logo size={26} />);
    const all = nodes(view.toJSON());
    // react-native-svg hands the view box to the native view as vbWidth / vbHeight.
    const svg = all.find((node) => node.props.vbWidth !== undefined);
    expect(svg?.props).toEqual(expect.objectContaining({ vbWidth: 68, vbHeight: 68 }));
    const rects = all.filter((node) => node.type.includes("Rect")).map((node) => node.props);
    const { bars, cursor } = fitLogo(68);
    for (const part of [...bars, cursor]) {
      expect([part.x, part.y, part.width, part.height].every(Number.isInteger)).toBe(true);
      expect(rects).toContainEqual(
        expect.objectContaining({ x: part.x, y: part.y, width: part.width, height: part.height }),
      );
    }
  });
});
