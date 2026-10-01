import { copyDraft, errorText, presetChars, presetProblems, sampleProblem } from "./preset-drafts";

const WEEKLY = {
  id: "7e57ab1e-0b0e-4c0d-9e5e-7e57ab1e0b0e",
  name: "Weekly",
  prompt: "整理成周报：按项目分组。",
  created_at_ms: 1,
  updated_at_ms: 1,
};

describe("preset drafts (desktop and phone editors)", () => {
  it("checks a draft in the core's rules", () => {
    expect(presetProblems({ name: " ", prompt: "" }, [])).toEqual({
      name: { text: "请填写名称", missing: true },
      prompt: { text: "请填写提示词", missing: true },
    });
    expect(presetProblems({ name: "字".repeat(25), prompt: "x".repeat(4001) }, [])).toEqual({
      name: { text: "名称最多 24 个字符", missing: false },
      prompt: { text: "提示词最多 4000 个字符", missing: false },
    });
    expect(presetProblems({ name: " WEEKLY ", prompt: "x" }, [WEEKLY]).name?.text).toBe(
      "已有名为「Weekly」的预设",
    );
    expect(presetProblems({ name: "周报", prompt: "x" }, [WEEKLY])).toEqual({});
  });

  it("counts characters as the core does and checks the trial sample", () => {
    expect(presetChars("😀字")).toBe(2);
    expect(sampleProblem("  ")).toBe("请输入示例文字");
    expect(sampleProblem("字".repeat(2001))).toBe("示例文字最多 2000 个字符");
    expect(sampleProblem("你好")).toBeUndefined();
  });

  it("names a copy of a built-in preset and words a command's rejection", () => {
    expect(copyDraft("notes", "正文")).toEqual({ name: "要点纪要（副本）", prompt: "正文" });
    expect(errorText(new Error("plain words"))).toBe("plain words");
    expect(errorText("not an error")).toBe("not an error");
  });
});
