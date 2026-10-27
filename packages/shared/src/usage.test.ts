import { describe, expect, it } from "vitest";
import { startOfDay } from "./history-stats";
import { sampleUsage } from "./mock-backend";
import { emptyUsageView, type QuotaReport, type UsageView } from "./schema";
import {
  audioAmount,
  currentQuota,
  dailyResetLine,
  nextBeijingMidnightMs,
  othersToday,
  providerUseAmount,
  resetCountdown,
  resetLabel,
  serviceUsage,
  tighter,
  usageAmount,
  usageDayLabel,
  usageInline,
  usageLeft,
  usageNotice,
  usageOf,
  usageOriginName,
  usageOriginNote,
  usagePurposeName,
  usagePurposeNote,
  usageSummary,
  usageValue,
  usageWeek,
  usableFallback,
  usesBuiltin,
} from "./usage";

/** 2026-10-27 10:00 in Beijing (02:00 UTC). */
const NOW = Date.UTC(2026, 9, 27, 2, 0);
/** The next Beijing midnight, Unix seconds. */
const RESET = Date.UTC(2026, 9, 27, 16, 0) / 1000;

function report(over: Partial<QuotaReport> = {}): QuotaReport {
  return {
    mode: "limit",
    basis: "device",
    day: "2026-10-27",
    reset_at: RESET,
    asr: { used_seconds: 23 * 60 + 40, limit_seconds: 3600 },
    refine: { used: 41, limit: 100 },
    ...over,
  };
}

function view(over: Partial<UsageView> = {}): UsageView {
  return { ...emptyUsageView(), builtin: true, quota: report(), ...over };
}

describe("serviceUsage", () => {
  it("reads each service against its limit in its own unit", () => {
    const asr = serviceUsage(view(), "asr", NOW);
    expect(asr).toMatchObject({ tone: "ok", used: 23, limit: 60, left: 36, leftUnderOne: false });
    expect(asr.share).toBeCloseTo(1420 / 3600);
    expect(asr.resetAtMs).toBe(RESET * 1000);
    const llm = serviceUsage(view(), "llm", NOW);
    expect(llm).toMatchObject({ tone: "ok", used: 41, limit: 100, left: 59, share: 0.41 });
  });

  it("turns amber at 80 % and red at the limit", () => {
    const at = (used_seconds: number) =>
      serviceUsage(
        view({ quota: report({ asr: { used_seconds, limit_seconds: 3600 } }) }),
        "asr",
        NOW,
      );
    expect(at(2879).tone).toBe("ok");
    expect(at(2880).tone).toBe("near");
    expect(at(3599)).toMatchObject({ tone: "near", left: 0, leftUnderOne: true });
    expect(at(3600)).toMatchObject({ tone: "out", left: 0, leftUnderOne: false, share: 1 });
    expect(at(4000)).toMatchObject({ tone: "out", used: 66, share: 1 });
    const zero = serviceUsage(
      view({ quota: report({ refine: { used: 0, limit: 0 } }) }),
      "llm",
      NOW,
    );
    expect(zero).toMatchObject({ tone: "out", share: 1 });
  });

  it("has no limit in the counting period", () => {
    const counting = view({ quota: report({ mode: "count" }) });
    expect(serviceUsage(counting, "asr", NOW)).toMatchObject({ tone: "unlimited", used: 23 });
    expect(serviceUsage(counting, "asr", NOW).limit).toBeUndefined();
    const noLimit = view({ quota: report({ refine: { used: 3, limit: null } }) });
    expect(serviceUsage(noLimit, "llm", NOW)).toMatchObject({ tone: "unlimited", used: 3 });
  });

  it("is unknown when the service could not be asked, and pending before its first report", () => {
    expect(serviceUsage(view({ unavailable: true }), "asr", NOW).tone).toBe("unknown");
    const { quota: _none, ...noReport } = view();
    expect(serviceUsage({ ...noReport }, "llm", NOW).tone).toBe("pending");
  });

  it("starts a new day at the reset, before the service says so", () => {
    const past = RESET * 1000 + 1;
    expect(serviceUsage(view(), "asr", past)).toMatchObject({ tone: "ok", used: 0, limit: 60 });
    expect(currentQuota(report(), past)?.reset_at).toBe(RESET + 86_400);
    expect(currentQuota(report(), NOW)).toEqual(report());
    expect(currentQuota(undefined, NOW)).toBeUndefined();
  });

  it("picks the tighter service", () => {
    const ok = serviceUsage(view(), "asr", NOW);
    const near = serviceUsage(
      view({ quota: report({ refine: { used: 90, limit: 100 } }) }),
      "llm",
      NOW,
    );
    expect(tighter(ok, near)).toBe(near);
    expect(tighter(near, ok)).toBe(near);
    const llm = serviceUsage(view(), "llm", NOW);
    expect(tighter(ok, llm)).toBe(llm);
  });
});

describe("usesBuiltin", () => {
  it("follows the provider of each service", () => {
    const engines = { asr_provider: "builtin", llm_provider: "groq" } as const;
    expect(usesBuiltin(engines as never, "asr")).toBe(true);
    expect(usesBuiltin(engines as never, "llm")).toBe(false);
  });
});

describe("texts", () => {
  it("names amounts, numbers and what is left", () => {
    const asr = serviceUsage(view(), "asr", NOW);
    expect(usageAmount("asr", 23)).toBe("23 分钟");
    expect(usageAmount("llm", 1, "en")).toBe("1 use");
    expect(usageAmount("llm", 2, "en")).toBe("2 uses");
    expect(usageOf(asr)).toBe("23 / 60 分钟");
    expect(usageOf(asr, "en")).toBe("23 / 60 min");
    expect(usageLeft(asr)).toBe("剩余 36 分钟");
    const under = serviceUsage(
      view({ quota: report({ asr: { used_seconds: 3570, limit_seconds: 3600 } }) }),
      "asr",
      NOW,
    );
    expect(usageLeft(under)).toBe("剩余不到 1 分钟");
    expect(usageValue(under)).toBe("剩余不到 1 分钟");
  });

  it("says each state on a row and on one line", () => {
    const at = (quota: Partial<QuotaReport>, extra: Partial<UsageView> = {}) =>
      serviceUsage(view({ quota: report(quota), ...extra }), "asr", NOW);
    const ok = at({});
    const near = at({ asr: { used_seconds: 52 * 60, limit_seconds: 3600 } });
    const out = at({ asr: { used_seconds: 3600, limit_seconds: 3600 } });
    const counting = at({ mode: "count" });
    const unknown = at({}, { unavailable: true });
    const { quota: _none, ...noReport } = view();
    const pending = serviceUsage(noReport, "asr", NOW);
    expect([ok, near, out, counting, unknown, pending].map((u) => usageValue(u))).toEqual([
      "23 / 60 分钟",
      "剩余 8 分钟",
      "已用完",
      "23 分钟",
      "无法获取",
      "—",
    ]);
    const reset = resetLabel(RESET * 1000, NOW);
    expect([ok, near, out, counting, unknown, pending].map((u) => usageInline(u, NOW))).toEqual([
      "今日 23 / 60 分钟",
      "今日剩余 8 分钟",
      `今日额度已用完 · ${reset}`,
      "今日已用 23 分钟 · 暂不限量",
      "用量暂时无法获取",
      "正在获取用量…",
    ]);
    const under = at({ asr: { used_seconds: 3590, limit_seconds: 3600 } });
    expect(usageInline(under, NOW)).toBe("今日剩余不到 1 分钟");
    expect(usageInline(near, NOW, "en")).toBe("8 min left today");
  });

  it("says when the day starts again in local time", () => {
    const today = startOfDay(NOW);
    const at = (h: number, m = 0) => today + (h * 60 + m) * 60_000;
    expect(resetLabel(at(23, 30), at(10))).toBe("今天 23:30 重置");
    expect(resetLabel(at(24), at(10))).toBe("明天 00:00 重置");
    expect(resetLabel(at(24 + 16), at(10), "en")).toBe("Resets tomorrow at 16:00");
    expect(resetLabel(at(48 + 1), at(10))).toBe("2 天后 01:00 重置");
    expect(resetCountdown(at(24), at(17, 48))).toBe("还有 6 小时 12 分");
    expect(resetCountdown(at(24), at(23, 59) + 30_000)).toBe("不到 1 分钟");
    expect(resetCountdown(at(24), at(25))).toBe("不到 1 分钟");
    expect(resetCountdown(at(11), at(10), "en")).toBe("in 1 h");
  });

  it("says the daily reset as this machine's clock shows Beijing's midnight", () => {
    // Beijing's midnight on this machine's clock: 00:00 in China, 16:00 in UTC (the CI runners).
    const here = new Date(RESET * 1000);
    const clock = `${String(here.getHours()).padStart(2, "0")}:${String(here.getMinutes()).padStart(2, "0")}`;
    const expected =
      clock === "00:00"
        ? "内置服务每天 00:00（北京时间）重置"
        : `内置服务每天北京时间 00:00（本机时间 ${clock}）重置`;
    expect(dailyResetLine(NOW)).toBe(expected);
    expect(nextBeijingMidnightMs(NOW)).toBe(RESET * 1000);
    expect(nextBeijingMidnightMs(RESET * 1000)).toBe((RESET + 86_400) * 1000);
  });

  it("names origins, purposes and other providers' use", () => {
    expect(usageOriginName({ kind: "phone", name: "Pixel 8" })).toBe("Pixel 8");
    expect(usageOriginName({ kind: "this_device" })).toBe("这台电脑");
    expect(usageOriginNote({ kind: "phone", name: "Pixel 8" })).toBe("手机发来的录音");
    expect(usageOriginName({ kind: "serve" }, "en")).toBe("Local speech service");
    expect([usagePurposeName("refine"), usagePurposeName("edit")]).toEqual(["AI 润色", "语音编辑"]);
    expect(usagePurposeNote("edit")).toBe("用语音修改选中的文字");
    expect(usagePurposeNote("refine", "en")).toBe("Cleaning up dictated text");
    expect(audioAmount(0)).toBe("0 分钟");
    expect(audioAmount(20)).toBe("不到 1 分钟");
    expect(audioAmount(38 * 60 + 59)).toBe("38 分钟");
    expect(audioAmount(2 * 3600 + 10 * 60 + 5)).toBe("2 小时 10 分");
    expect(audioAmount(3600)).toBe("1 小时");
    expect(providerUseAmount({ provider: "groq", service: "llm", amount: 312 })).toBe("312 次");
    expect(providerUseAmount({ provider: "local", service: "asr", amount: 7800 })).toBe(
      "2 小时 10 分",
    );
  });
});

describe("usageSummary", () => {
  it("says both services in one line for the phone's settings row", () => {
    expect(usageSummary(view(), NOW)).toBe("识别 23 / 60 分钟 · 润色 41 / 100 次");
    const near = view({
      quota: report({ asr: { used_seconds: 52 * 60, limit_seconds: 3600 }, refine: { used: 100, limit: 100 } }),
    });
    expect(usageSummary(near, NOW)).toBe("识别剩余 8 分钟 · 润色已用完");
    expect(usageSummary(view({ quota: report({ mode: "count" }) }), NOW)).toBe(
      "识别 23 分钟 · 润色 41 次 · 暂不限量",
    );
    expect(usageSummary(view({ unavailable: true }), NOW)).toBe("用量暂时无法获取");
    expect(usageSummary(view({ quota: undefined }), NOW)).toBe("正在获取用量…");
    expect(usageSummary(view(), NOW, "en")).toBe("Speech 23 / 60 min · Polish 41 / 100 uses");
  });
});

describe("record", () => {
  it("sums today's other providers of one service", () => {
    const v = view({
      others_today: [
        { provider: "local", service: "asr", amount: 1380 },
        { provider: "aliyun", service: "asr", amount: 60 },
        { provider: "groq", service: "llm", amount: 41 },
      ],
    });
    expect(othersToday(v, "asr")).toEqual({ amount: 1440, provider: "local" });
    expect(othersToday(v, "llm")).toEqual({ amount: 41, provider: "groq" });
    expect(othersToday(view(), "llm")).toEqual({ amount: 0 });
  });

  it("charts seven days with today as the service reported it", () => {
    const days = [
      "2026-10-21",
      "2026-10-22",
      "2026-10-23",
      "2026-10-24",
      "2026-10-25",
      "2026-10-26",
      "2026-10-27",
    ];
    const v = view({ days: days.map((day, i) => ({ day, asr_seconds: i * 600, refine: i * 10 })) });
    const asr = usageWeek(v, "asr", NOW);
    expect(asr.map((d) => d.value)).toEqual([0, 10, 20, 30, 40, 50, 60]);
    expect(asr.filter((d) => d.today).map((d) => d.day)).toEqual(["2026-10-27"]);
    // The service counts more than this device recorded (another app's takes): its number.
    const llm = usageWeek(v, "llm", NOW);
    expect(llm.at(-1)?.value).toBe(60);
    const more = view({ ...v, quota: report({ refine: { used: 75, limit: 100 } }) });
    expect(usageWeek(more, "llm", NOW).at(-1)?.value).toBe(75);
    expect(usageWeek({ ...v, unavailable: true }, "llm", NOW).at(-1)?.value).toBe(60);
  });

  it("names a chart's days by weekday and today as 今天", () => {
    expect(usageDayLabel("2026-10-27", true)).toBe("今天");
    expect(usageDayLabel("2026-10-21", false)).toBe("周三");
    expect(usageDayLabel("2026-10-25", false, "en")).toBe("Sun");
  });
});

describe("usageNotice", () => {
  it("shows at 80 % and at the limit while the built-in service is in use, unless closed today", () => {
    const near = view({ quota: report({ asr: { used_seconds: 3000, limit_seconds: 3600 } }) });
    const nearAsr = serviceUsage(near, "asr", NOW);
    expect(usageNotice(nearAsr, near, true)).toBe("asr_near");
    expect(usageNotice(nearAsr, near, false)).toBeUndefined();
    expect(usageNotice(nearAsr, { ...near, dismissed: ["asr_near"] }, true)).toBeUndefined();
    expect(usageNotice(nearAsr, { ...near, builtin: false }, true)).toBeUndefined();
    const out = view({ quota: report({ refine: { used: 100, limit: 100 } }) });
    expect(usageNotice(serviceUsage(out, "llm", NOW), out, true)).toBe("refine_out");
    expect(usageNotice(serviceUsage(view(), "llm", NOW), view(), true)).toBeUndefined();
  });
});

describe("usableFallback", () => {
  it("is the first fallback model that can run now, while the chain is in use", () => {
    const model = (over: object) => ({ provider: "aliyun" as const, model: "m", ...over });
    expect(usableFallback(undefined, NOW)).toBeUndefined();
    const off = { enabled: true, in_use: false, models: [model({})] };
    expect(usableFallback(off, NOW)).toBeUndefined();
    const chain = {
      enabled: true,
      in_use: true,
      models: [
        model({ model: "a", issue: "key_missing" as const }),
        model({ model: "b", skip: "duplicate" as const }),
        model({ model: "c", retry_at_ms: NOW + 1 }),
        model({ model: "d", retry_at_ms: NOW }),
      ],
    };
    expect(usableFallback(chain, NOW)?.model).toBe("d");
    expect(usableFallback({ ...chain, models: chain.models.slice(0, 3) }, NOW)).toBeUndefined();
  });
});

describe("sampleUsage (the previews' days)", () => {
  it("gives each state its numbers, a week ending today and today's breakdown", () => {
    const states = ["ok", "near", "out", "count", "unknown"] as const;
    const tones = states.map((state) => {
      const v = { ...emptyUsageView(), ...sampleUsage(state, NOW) };
      expect(v.days).toHaveLength(7);
      expect(v.days.at(-1)?.day).toBe("2026-10-27");
      expect(v.quota?.reset_at).toBe(RESET);
      expect(v.asr_origins.reduce((sum, o) => sum + o.seconds, 0)).toBe((v.quota?.asr.used_seconds ?? 0));
      expect(v.refine_purposes.reduce((sum, p) => sum + p.count, 0)).toBe(v.quota?.refine.used);
      return [serviceUsage(v, "asr", NOW).tone, serviceUsage(v, "llm", NOW).tone];
    });
    expect(tones).toEqual([
      ["ok", "ok"],
      ["near", "near"],
      ["out", "out"],
      ["unlimited", "unlimited"],
      ["unknown", "unknown"],
    ]);
    expect(sampleUsage().builtin).toBe(true);
  });
});
