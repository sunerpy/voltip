import { diffSegments, diffTokens } from "./refine";

describe("diffSegments", () => {
  it("produces same/del/add runs at token level", () => {
    expect(diffTokens("ab c，d")).toEqual(["ab ", "c", "，", "d"]);
    const segs = diffSegments({ rawText: "甲乙丙", text: "甲乙丁" });
    expect(segs).toEqual([
      { kind: "same", text: "甲乙" },
      { kind: "del", text: "丙" },
      { kind: "add", text: "丁" },
    ]);
    expect(diffSegments({ rawText: "", text: "xy" })).toEqual([{ kind: "add", text: "xy" }]);
    expect(diffSegments({ rawText: "xy", text: "" })).toEqual([{ kind: "del", text: "xy" }]);
    expect(diffSegments({ rawText: "same", text: "same" })).toEqual([
      { kind: "same", text: "same" },
    ]);
    const real = diffSegments({
      rawText: "这个函数的返回值类型改成 option string 空字符串不要再出现",
      text: "这个函数的返回值类型改成 Option<String>，空字符串不要再出现。",
    });
    expect(real.find((s) => s.kind === "del")?.text.trim()).toBe("option string");
    expect(real.some((s) => s.kind === "add" && s.text.includes("Option<String>"))).toBe(true);
    expect(real.some((s) => s.kind === "add" && s.text.includes("。"))).toBe(true);
  });
});
