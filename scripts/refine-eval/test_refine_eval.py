"""Tests for scripts/refine-eval/refine_eval.py (stdlib unittest, no network).

Run: python3 -m unittest discover -s scripts/refine-eval -p 'test_*.py'
"""

from __future__ import annotations

import hashlib
import argparse
import importlib.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("refine_eval", HERE / "refine_eval.py")
assert SPEC is not None and SPEC.loader is not None
ev = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = ev
SPEC.loader.exec_module(ev)


def run(n: int, status: str = "refined", answer: str = "x") -> dict:
    return {"run": n, "status": status, "answer": answer, "prompt_tokens": 100}


class TextChecks(unittest.TestCase):
    def test_norm_folds_width_case_and_whitespace(self) -> None:
        self.assertEqual(ev.norm("  Ｇｏ  语言 \n60 秒 "), "go语言60秒")
        self.assertEqual(ev.letters("嗯，那个… So, UM!"), "嗯那个soum")

    def test_any_all_and_none(self) -> None:
        point = {"any": ["四十五天", "45天"], "none": ["三十天", "30天"]}
        self.assertEqual(ev.check_point(point, "付款期限改成 45 天。", ""), (True, ""))
        kept, note = ev.check_point(point, "付款期限从 30 天改成 45 天。", "")
        self.assertFalse(kept)
        self.assertIn("30天", note)
        kept, note = ev.check_point({"any": ["四十五天", "45天"]}, "付款期限改了。", "")
        self.assertFalse(kept)
        self.assertIn("四十五天 / 45天", note)
        both = {"all": ["八分钟|8分钟", "二十分钟|20分钟"]}
        self.assertTrue(ev.check_point(both, "从 20 分钟降到 8 分钟", "")[0])
        self.assertFalse(ev.check_point(both, "降到 8 分钟", "")[0])
        self.assertTrue(ev.check_point({"any": ["docker compose up -d"]}, "运行 `docker compose up -d`", "")[0], "whitespace never matters")
        self.assertTrue(ev.check_point({"any": ["third-floor"]}, "the third\u2011floor room", "")[0], "a non-breaking hyphen is a hyphen")

    def test_verbatim_keeps_every_letter_and_digit(self) -> None:
        point = {"verbatim": True}
        source = "嗯那个我觉得这个方案可以然后然后我们下周再细聊吧"
        self.assertTrue(ev.check_point(point, "嗯，那个，我觉得这个方案可以，然后然后，我们下周再细聊吧。", source)[0])
        self.assertFalse(ev.check_point(point, "我觉得这个方案可以，然后我们下周再细聊吧。", source)[0])
        self.assertTrue(ev.check_point(point, "So, um, I think...", "so um I think")[0], "a capital at the start is not a change")

    def test_reasoning_is_stripped_as_the_built_in_service_never_shows_it(self) -> None:
        self.assertEqual(ev.strip_reasoning("<reasoning>想一想</reasoning>明天见。"), ("明天见。", 3))
        self.assertEqual(ev.strip_reasoning("  <reasoning>a\nb</reasoning>\n\n好的"), ("\n\n好的", 3))
        self.assertEqual(ev.strip_reasoning("<reasoning>还没想完"), ("", len("<reasoning>还没想完")), "cut off while reasoning: no answer")
        self.assertEqual(ev.strip_reasoning("没有推理"), ("没有推理", 0))

    def test_generation_status(self) -> None:
        self.assertEqual(ev.generation_status({"finish_reason": "stop"}, "好"), "refined")
        self.assertEqual(ev.generation_status({"finish_reason": "length"}, "好"), "truncated")
        self.assertEqual(ev.generation_status({"finish_reason": "stop"}, ""), "empty")
        self.assertEqual(ev.generation_status({"error": "HTTP 400"}, None), "error")


class Judge(unittest.TestCase):
    VERDICT = {
        "outputs": [
            {"label": "A", "points": {"p1": {"kept": True, "note": ""}, "p2": {"kept": False, "note": "丢了条件"}}, "purpose_ok": True, "purpose_issues": []},
            {"label": "B", "points": {"p1": {"kept": True}, "p2": {"kept": True}}, "purpose_ok": False, "purpose_issues": ["回答了问题"]},
        ]
    }

    def test_a_verdict_is_read_from_the_answer(self) -> None:
        text = "好的，结果如下：\n```json\n" + json.dumps(self.VERDICT, ensure_ascii=False) + "\n```"
        verdicts = ev.parse_verdict(text, ["A", "B"], ["p1", "p2"])
        self.assertEqual(verdicts[0]["points"]["p2"], {"kept": False, "note": "丢了条件"})
        self.assertEqual((verdicts[1]["purpose_ok"], verdicts[1]["purpose_issues"]), (False, ["回答了问题"]))

    def test_an_incomplete_verdict_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            ev.parse_verdict(json.dumps(self.VERDICT), ["A", "B", "C"], ["p1", "p2"])
        with self.assertRaises(ValueError):
            ev.parse_verdict(json.dumps(self.VERDICT), ["A", "B"], ["p1", "p2", "p3"])
        with self.assertRaises(ValueError):
            ev.parse_verdict("没有 JSON", ["A"], ["p1"])
        broken = {"outputs": [{"label": "A", "points": {"p1": {"kept": "yes"}}, "purpose_ok": True}]}
        with self.assertRaises(ValueError):
            ev.parse_verdict(json.dumps(broken), ["A"], ["p1"])

    def test_the_message_carries_the_purpose_the_points_and_each_output(self) -> None:
        sample = {"text": "原文", "points": [{"id": "p1", "point": "要点一"}, {"id": "p2", "point": "要点二"}]}
        message = ev.judge_message(sample, {"name": "校对", "purpose": "只校对"}, ["结果甲", "结果乙"])
        for part in ("预设：校对", "预设用途：只校对", "原文", "- p1：要点一", "- p2：要点二", "结果 A：", "结果甲", "结果 B：", "结果乙"):
            self.assertIn(part, message)

    def test_a_checked_point_is_kept_only_when_the_check_and_the_judge_both_find_it(self) -> None:
        sample = {
            "text": "付款期限改成三十天不对是四十五天登录问题交给小陈",
            "points": [{"id": "p1", "point": "45 天", "any": ["45天"]}, {"id": "p2", "point": "语义"}, {"id": "p3", "point": "登录问题交给小陈", "any": ["小陈"]}],
        }
        runs = [run(1, answer="改成 45 天，登录问题交给小陈。"), run(2, "truncated"), run(3, answer="改成 30 天，登录问题交给小陈。"), run(4, answer="改成 45 天，登录问题交给老刘，报表交给小陈。")]
        kept = {"kept": True, "note": ""}
        verdicts = [
            {"points": {"p1": kept, "p2": kept, "p3": kept}, "purpose_ok": True, "purpose_issues": []},
            {"points": {"p1": kept, "p2": {"kept": False, "note": "缺"}, "p3": kept}, "purpose_ok": False, "purpose_issues": ["加了内容"]},
            {"points": {"p1": kept, "p2": kept, "p3": {"kept": False, "note": "交给了老刘"}}, "purpose_ok": True, "purpose_issues": []},
        ]
        out = ev.evaluate_runs(sample, runs, verdicts)
        self.assertEqual(out[0]["points"]["p1"], {"kept": True, "by": "rule", "note": "", "rule": True, "judge": True, "judge_note": ""})
        self.assertNotIn("points", out[1], "a fallback keeps the raw text: nothing to judge")
        self.assertFalse(out[2]["points"]["p1"]["kept"], "the check finds the value the speaker took back")
        self.assertEqual((out[2]["points"]["p1"]["by"], out[2]["points"]["p1"]["judge"]), ("rule", True), "though the judge let it pass")
        self.assertEqual(out[2]["points"]["p2"], {"kept": False, "by": "judge", "note": "缺"})
        self.assertEqual(out[2]["purpose_issues"], ["加了内容"])
        self.assertEqual(out[3]["points"]["p3"], {"kept": False, "by": "judge", "note": "交给了老刘", "rule": True, "judge": False, "judge_note": "交给了老刘"}, "the name is there, but not where it was said")

    def test_a_verbatim_check_alone_decides_its_point(self) -> None:
        sample = {"text": "so um ship it on friday", "points": [{"id": "p1", "point": "每个词都保留", "verbatim": True}]}
        verdict = [{"points": {"p1": {"kept": False, "note": "Friday 改成了大写"}}, "purpose_ok": False, "purpose_issues": ["改了大小写"]}]
        out = ev.evaluate_runs(sample, [run(1, answer="so, um, ship it on Friday.")], verdict)
        self.assertTrue(out[0]["points"]["p1"]["kept"], "case is the purpose's business; no word was dropped")
        self.assertFalse(out[0]["purpose_ok"])


class FakeJudge:
    """Answers every judge call with the same verdict for each label, the point kept unless the answer
    says 漏, and keeps the messages it was sent."""

    def __init__(self) -> None:
        self.messages: list[str] = []

    def judge(self, system: str, message: str) -> tuple[str, int, int]:
        self.messages.append(message)
        outputs = []
        for block in message.split("结果 ")[1:]:
            label, rest = block.split("：", 1)
            answer = rest.split("<<<", 1)[1].split(">>>", 1)[0]
            outputs.append({"label": label, "points": {"p1": {"kept": "漏" not in answer, "note": "" if "漏" not in answer else "漏了"}}, "purpose_ok": True, "purpose_issues": []})
        return json.dumps({"outputs": outputs}, ensure_ascii=False), 10, 5


class Pairing(unittest.TestCase):
    SAMPLE = {"id": "s", "unit": "preset:chat", "text": "晚点到", "points": [{"id": "p1", "point": "晚点到"}]}
    UNIT = {"name": "口语聊天", "purpose": "聊天"}

    def test_both_sets_are_judged_in_one_call_each_answer_once(self) -> None:
        after = [run(1, answer="晚点到"), run(2, answer="漏"), run(3, "truncated")]
        before = [run(1, answer="晚点到"), run(2, answer="晚点到"), run(3, answer="晚点到啦")]
        judge = FakeJudge()
        verdicts, usage = ev.judge_sample(judge, self.SAMPLE, self.UNIT, [after, before], "s:120b")
        self.assertEqual(len(judge.messages), 1, "one call for both")
        self.assertEqual(judge.messages[0].count("<<<"), 3, "晚点到 / 漏 / 晚点到啦, each once")
        self.assertEqual([v["points"]["p1"]["kept"] for v in verdicts[0]], [True, False], "aligned with the refined runs")
        self.assertEqual([v["points"]["p1"]["kept"] for v in verdicts[1]], [True, True, True])
        self.assertIs(verdicts[0][0], verdicts[1][0], "the same answer, the same verdict")
        self.assertEqual(usage, {"calls": 1, "input_tokens": 10, "output_tokens": 5})
        again = FakeJudge()
        ev.judge_sample(again, self.SAMPLE, self.UNIT, [after, before], "s:120b")
        self.assertEqual(again.messages, judge.messages, "the shuffled order is reproducible")

    def test_a_sample_without_a_refined_answer_needs_no_call(self) -> None:
        judge = FakeJudge()
        verdicts, usage = ev.judge_sample(judge, self.SAMPLE, self.UNIT, [[run(1, "truncated")], [run(1, "empty")]], "s:20b")
        self.assertEqual((verdicts, usage["calls"], judge.messages), ([None, None], 0, []))

    def test_a_paired_verdict_is_redone_when_its_partner_changed(self) -> None:
        sample = {"id": "a", "text": "好", "points": [{"id": "p1", "point": "好"}]}
        runs = [generation("a", 1, "s")]
        partner = [generation("a", 1, "t")]
        record = {"id": "a", "sample_sha": ev.sample_sha(sample), "answers_sha": ev.answers_sha(runs), "paired_with": ev.answers_sha(partner)}
        self.assertFalse(ev.needs_judging(record, sample, runs, False, ev.answers_sha(partner)))
        self.assertTrue(ev.needs_judging(record, sample, runs, False, ev.answers_sha([generation("a", 1, "t", content="变了")])))
        self.assertTrue(ev.needs_judging(record, sample, runs, False), "judged with a partner, asked without one")


class PairedRun(unittest.TestCase):
    """judge --pair-with over a work directory and a saved results directory, as `all` runs it."""

    SAMPLE = {"id": "s", "unit": "preset:chat", "text": "晚点到", "points": [{"id": "p1", "point": "晚点到"}]}
    DATA = {"units": {"preset:chat": {"name": "口语聊天", "purpose": "聊天"}}, "samples": [SAMPLE]}

    def judge(self, work: Path, saved: Path, judges: list) -> None:
        def bedrock(ledger: object, profile: str, region: str) -> FakeJudge:
            judges.append(FakeJudge())
            return judges[-1]

        args = argparse.Namespace(out=str(work), pair_with=str(saved), models="120b", only=None, workers=1, max_calls=10, profile="p", region="r", force=False)
        with mock.patch.multiple(ev, load_samples=lambda *a: self.DATA, clean_answers=lambda xs: list(xs), Bedrock=bedrock):
            ev.cmd_judge(args)

    def test_a_work_directory_is_judged_beside_saved_results_and_both_are_written(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            work, saved = Path(tmp) / "work", Path(tmp) / "before"
            ev.write_jsonl(work / "generations-120b.jsonl", [generation("s", 1, "t", content="晚点到"), generation("s", 2, "t", content="漏")])
            old = {"id": "s", "unit": "preset:chat", "sample_sha": ev.sample_sha(self.SAMPLE), "answers_sha": "old", "paired_with": None}
            old["runs"] = ev.evaluate_runs(self.SAMPLE, [run(1, answer="晚点到"), run(2, answer="晚点到啦")], None)
            ev.write_jsonl(saved / "120b.jsonl", [old])
            judges: list[FakeJudge] = []
            self.judge(work, saved, judges)
            self.assertEqual(len(judges[0].messages), 1, "one call for both sides")
            self.assertEqual(judges[0].messages[0].count("<<<"), 3, "晚点到 once")
            mine, theirs = ev.read_jsonl(work / "verdicts-120b.jsonl")[0], ev.read_jsonl(saved / "120b.jsonl")[0]
            self.assertEqual((mine["paired_with"], theirs["paired_with"], theirs["answers_sha"]), ("old", mine["answers_sha"], "old"))
            self.assertEqual([r["points"]["p1"]["kept"] for r in mine["runs"]], [True, False])
            self.assertEqual([r["points"]["p1"]["kept"] for r in theirs["runs"]], [True, True])
            self.assertEqual(theirs["runs"][1]["answer"], "晚点到啦", "the saved answers stay")
            self.judge(work, saved, judges)
            self.assertEqual(judges[1].messages, [], "nothing changed, nothing judged again")

    def test_a_sample_the_saved_results_lack_is_judged_alone(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            work, saved = Path(tmp) / "work", Path(tmp) / "before"
            ev.write_jsonl(work / "generations-120b.jsonl", [generation("s", 1, "t", content="晚点到")])
            ev.write_jsonl(saved / "120b.jsonl", [{"id": "other", "unit": "preset:chat", "answers_sha": "x", "runs": []}])
            judges: list[FakeJudge] = []
            self.judge(work, saved, judges)
            self.assertIsNone(ev.read_jsonl(work / "verdicts-120b.jsonl")[0]["paired_with"])


class Recheck(unittest.TestCase):
    SAMPLE = {"id": "s", "text": "肝肾功能", "points": [{"id": "p1", "point": "肝肾功能", "any": ["肝肾功能"]}, {"id": "p2", "point": "语义"}]}

    def judged(self) -> dict:
        runs = [run(1, answer="肝功能、肾功能"), run(2, "truncated")]
        verdicts = [{"points": {"p1": {"kept": True, "note": ""}, "p2": {"kept": False, "note": "缺"}}, "purpose_ok": True, "purpose_issues": []}]
        return {"id": "s", "unit": "scene:medical", "sample_sha": ev.sample_sha(self.SAMPLE), "runs": ev.evaluate_runs(self.SAMPLE, runs, verdicts)}

    def test_a_corrected_check_runs_again_over_the_stored_answers(self) -> None:
        record = self.judged()
        self.assertFalse(record["runs"][0]["points"]["p1"]["kept"])
        fixed = {**self.SAMPLE, "points": [{"id": "p1", "point": "肝肾功能", "all": ["肝功能|肝肾功能", "肾功能|肝肾功能"]}, self.SAMPLE["points"][1]]}
        again = ev.recheck_record(record, fixed)
        self.assertEqual(again["runs"][0]["points"]["p1"], {"kept": True, "by": "rule", "note": "", "rule": True, "judge": True, "judge_note": ""})
        self.assertEqual(again["runs"][0]["points"]["p2"], record["runs"][0]["points"]["p2"], "the judge's verdicts stay")
        self.assertEqual(again["runs"][1], record["runs"][1])
        judged_only = {**self.SAMPLE, "points": [{"id": "p1", "point": "肝肾功能"}, self.SAMPLE["points"][1]]}
        self.assertEqual(ev.recheck_record(record, judged_only)["runs"][0]["points"]["p1"]["by"], "judge", "the judge's recorded opinion decides")

    def test_an_older_record_keeps_the_judges_opinion_without_its_note(self) -> None:
        sample = {"id": "s", "text": "交给小陈", "points": [{"id": "p1", "point": "交给小陈", "any": ["小陈"]}]}
        old = {"id": "s", "unit": "preset:notes", "runs": [{"run": 1, "status": "refined", "answer": "交给老刘，小陈不管", "points": {"p1": {"kept": True, "by": "rule", "note": "", "judge": False}}, "purpose_ok": True, "purpose_issues": []}]}
        point = ev.recheck_record(old, sample)["runs"][0]["points"]["p1"]
        self.assertEqual((point["kept"], point["by"], point["judge"]), (False, "judge", False))
        self.assertTrue(point["note"], "a note says the judge found it missing")

    def test_a_sample_the_judge_saw_differently_is_refused(self) -> None:
        changed = {**self.SAMPLE, "text": "肝功能"}
        with self.assertRaises(ev.EvalError):
            ev.recheck_record(self.judged(), changed)


def generation(sid: str, run_no: int, sha: str, content: str = "好", **extra: object) -> dict:
    return {"id": sid, "unit": "preset:chat", "run": run_no, "request_sha": sha, "finish_reason": "stop", "content": content, **extra}


class Integrity(unittest.TestCase):
    def test_canonical_json_is_the_eval_tools(self) -> None:
        # The value and the bytes of canonical_json_sorts_keys_at_every_level in crates/voltip-refine-eval.
        value = {"b": 1, "a": [{"d": 0.2, "c": "引号\"和\n换行"}], "e": None}
        self.assertEqual(ev.canonical(value), '{"a":[{"c":"引号\\"和\\n换行","d":0.2}],"b":1,"e":null}')

    def test_a_request_is_known_by_everything_but_its_model(self) -> None:
        body = {"model": "a", "messages": [{"role": "system", "content": "x"}], "temperature": 0.2}
        self.assertEqual(ev.request_sha(body), ev.request_sha({**body, "model": "b"}))
        self.assertNotEqual(ev.request_sha(body), ev.request_sha({**body, "temperature": 0.3}))

    def test_a_units_digest_is_the_eval_tools_over_its_requests(self) -> None:
        requests = [
            {"id": "b", "unit": "u", "body": {"model": "m", "x": 1}},
            {"id": "a", "unit": "u", "body": {"model": "n", "x": 2}},
            {"id": "c", "unit": "v", "body": {"x": 3}},
        ]
        digests = ev.unit_digests(requests)
        self.assertEqual(digests, ev.unit_digests(list(reversed(requests))), "the order does not matter")
        self.assertEqual(digests["u"], hashlib.sha256(b'a\n{"x":2}\nb\n{"x":1}\n').hexdigest(), "id, newline, request, newline; by id")
        self.assertEqual(ev.changed_units({"u": "1", "v": "2"}, {"u": "1", "v": "3", "w": "4"}), ["v", "w"])

    def test_only_answers_to_the_current_request_are_kept(self) -> None:
        rows = [generation("a", 1, "old"), generation("a", 2, "new"), generation("b", 1, "new", error="HTTP 500"), {**generation("b", 2, "x"), "request_sha": None}]
        self.assertEqual(ev.kept_generations(rows, {"a": "new", "b": "x"}), {("a", 2)})

    def test_a_verdict_is_redone_when_its_answers_or_its_sample_changed(self) -> None:
        sample = {"id": "a", "text": "好", "points": [{"id": "p1", "point": "好"}]}
        runs = [generation("a", 1, "s"), generation("a", 2, "s")]
        record = {"id": "a", "sample_sha": ev.sample_sha(sample), "answers_sha": ev.answers_sha(runs)}
        self.assertFalse(ev.needs_judging(record, sample, runs, force=False))
        self.assertTrue(ev.needs_judging(record, sample, runs, force=True))
        self.assertTrue(ev.needs_judging(None, sample, runs, force=False))
        self.assertTrue(ev.needs_judging(record, sample, [runs[0], generation("a", 2, "s", content="不好")], force=False))
        self.assertTrue(ev.needs_judging(record, {**sample, "text": "不好"}, runs, force=False))

    def test_saved_results_match_their_requests_and_answers(self) -> None:
        sample = {"id": "a", "unit": "preset:chat", "text": "好", "points": [{"id": "p1", "point": "好"}]}
        body = {"model": "m", "messages": [{"role": "system", "content": "x"}]}
        requests = [{"id": "a", "unit": "preset:chat", "body": body}]
        runs = [generation("a", 1, ev.request_sha(body))]
        verdicts = [{"id": "a", "unit": "preset:chat", "sample_sha": ev.sample_sha(sample), "answers_sha": ev.answers_sha(runs), "runs": []}]
        self.assertEqual(ev.check_work(requests, runs, verdicts, [sample]), ev.unit_digests(requests))
        cases = [
            ([generation("a", 1, "other")], verdicts, "other request"),
            (runs, [{**verdicts[0], "answers_sha": "x"}], "judged other answers"),
            (runs, [], "no verdict"),
        ]
        for gens, verdict_rows, why in cases:
            with self.assertRaises(ev.EvalError) as caught:
                ev.check_work(requests, gens, verdict_rows, [sample])
            self.assertIn(why, str(caught.exception))


class Samples(unittest.TestCase):
    def test_the_repository_samples_are_valid(self) -> None:
        data = ev.load_samples()
        self.assertEqual(len(ev.unit_order(data)), 14)
        for sample in data["samples"]:
            self.assertIn(sample["unit"], data["units"])

    def test_bad_samples_are_refused(self) -> None:
        good = {"units": {"preset:chat": {"name": "口语聊天", "purpose": "聊天"}}, "samples": [{"id": "a", "unit": "preset:chat", "text": "好", "points": [{"id": "p1", "point": "好"}]}]}
        ev.validate_samples(good)
        cases = [
            ({"samples": [{**good["samples"][0], "unit": "preset:notes"}]}, "no purpose"),
            ({"samples": [{**good["samples"][0], "points": [{"id": "p1", "point": "x"}, {"id": "p1", "point": "y"}]}]}, "unique"),
            ({"samples": [{**good["samples"][0], "points": [{"id": "p1", "point": "x", "any": []}]}]}, "non-empty list"),
            ({"samples": [{**good["samples"][0], "points": [{"id": "p1"}]}]}, "no description"),
            ({"samples": [good["samples"][0], good["samples"][0]]}, "twice"),
        ]
        for change, why in cases:
            with self.assertRaises(ev.EvalError) as caught:
                ev.validate_samples({**good, **change})
            self.assertIn(why, str(caught.exception))


class Ledger(unittest.TestCase):
    def test_a_run_stops_at_its_limit_and_every_call_is_written(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            ledger = ev.Ledger(Path(tmp) / "calls.jsonl", 2, "before")
            ledger.reserve()
            ledger.record("generate", "openai.gpt-oss-120b-1:0", "ok", 1_000_000, 1_000_000)
            ledger.reserve()
            ledger.record("judge", ev.JUDGE_MODEL, "ThrottlingException")
            with self.assertRaises(ev.BudgetExceeded):
                ledger.reserve()
            entries = ev.read_jsonl(Path(tmp) / "calls.jsonl")
            self.assertEqual([e["dir"] for e in entries], ["before", "before"])
            summary = ev.summarize_calls(entries)
            self.assertEqual(summary["calls"], 2)
            self.assertEqual(summary["models"][ev.JUDGE_MODEL]["failed"], 1)
            self.assertAlmostEqual(summary["usd"], 0.75)

    def test_rows_merge_by_key_the_later_one_winning(self) -> None:
        rows = ev.merge_rows([{"id": "b", "run": 1, "v": 1}, {"id": "a", "run": 2, "error": "x"}], [{"id": "a", "run": 2, "v": 2}], ("id", "run"))
        self.assertEqual(rows, [{"id": "a", "run": 2, "v": 2}, {"id": "b", "run": 1, "v": 1}])

    def test_the_aws_environment_never_reaches_the_clients(self) -> None:
        saved = {k: os.environ.get(k) for k in ev.AWS_ENV}
        try:
            for k in ev.AWS_ENV:
                os.environ[k] = "set-by-the-host"
            ev.drop_aws_env()
            self.assertFalse([k for k in ev.AWS_ENV if k in os.environ])
        finally:
            for k, v in saved.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v


def record(sid: str, unit: str, kept: list[list[bool]], statuses: list[str] | None = None, purpose: list[bool] | None = None) -> dict:
    runs = []
    for i, keeps in enumerate(kept):
        status = (statuses or ["refined"] * len(kept))[i]
        if status != "refined":
            runs.append({"run": i + 1, "status": status, "answer": ""})
            continue
        runs.append(
            {
                "run": i + 1,
                "status": status,
                "answer": f"答{i}",
                "prompt_tokens": 500,
                "points": {f"p{j + 1}": {"kept": k, "by": "judge", "note": "" if k else "缺了"} for j, k in enumerate(keeps)},
                "purpose_ok": (purpose or [True] * len(kept))[i],
                "purpose_issues": [] if (purpose or [True] * len(kept))[i] else ["回答了问题"],
            }
        )
    return {"id": sid, "unit": unit, "runs": runs}


class Report(unittest.TestCase):
    DATA = {
        "units": {"preset:prompt": {"name": "提示词优化", "purpose": "整理"}, "preset:chat": {"name": "口语聊天", "purpose": "聊天"}},
        "samples": [
            {"id": "prompt-01", "unit": "preset:prompt", "text": "帮我写脚本", "points": [{"id": "p1", "point": "写脚本"}, {"id": "p2", "point": "跳过空文件"}]},
            {"id": "chat-01", "unit": "preset:chat", "text": "晚点到", "points": [{"id": "p1", "point": "晚点到"}, {"id": "p2", "point": "七点半"}]},
        ],
    }

    def results(self, prompt: list[list[bool]], chat: list[list[bool]], digest: str = "1", **kw: object) -> dict:
        records = {"prompt-01": record("prompt-01", "preset:prompt", prompt, **kw), "chat-01": record("chat-01", "preset:chat", chat)}
        meta = {
            "commit": "abc",
            "generated_at": "2026-11-07",
            "runs": 3,
            "lengths": [{"unit": "preset:prompt", "chars": 1357, "limit": 4000}],
            "calls": {"calls": 9, "usd": 0.12, "models": {}},
            "digests": {"preset:prompt": digest, "preset:chat": "c"},
        }
        return {"meta": meta, "models": {"120b": records}, "prompts": {}}

    def test_the_goal_needs_every_unit_no_worse_and_the_total_lower(self) -> None:
        before = self.results([[True, False]] * 3, [[True, True]] * 3)
        after = self.results([[True, True]] * 3, [[True, True]] * 3)
        result = ev.compare(before, after, ev.unit_order(self.DATA), "120b")
        self.assertTrue(result["goal"])
        self.assertEqual((result["before"].omissions, result["after"].omissions), (3, 0))
        worse = self.results([[True, True]] * 3, [[True, False], [True, True], [True, True]])
        self.assertFalse(ev.compare(before, worse, ev.unit_order(self.DATA), "120b")["goal"], "口语聊天 got worse")
        same = ev.compare(before, before, ev.unit_order(self.DATA), "120b")
        self.assertFalse(same["goal"], "the total has to go down")

    def test_only_samples_both_sides_have_are_compared_and_the_pairing_is_named(self) -> None:
        before = self.results([[True, False]] * 3, [[True, True]] * 3)
        after = self.results([[True, True]] * 3, [[True, True]] * 3)
        after["models"]["120b"]["chat-02"] = record("chat-02", "preset:chat", [[False, False]] * 3)
        result = ev.compare(before, after, ev.unit_order(self.DATA), "120b")
        self.assertEqual((result["before"].omissions, result["after"].omissions), (3, 0), "chat-02 is only in after")
        for sid in ("prompt-01", "chat-01"):
            before["models"]["120b"][sid]["answers_sha"] = f"b-{sid}"
            after["models"]["120b"][sid]["paired_with"] = f"b-{sid}"
        data = {**self.DATA, "samples": [*self.DATA["samples"], {"id": "chat-02", "unit": "preset:chat", "text": "新样例", "points": [{"id": "p1", "point": "新"}, {"id": "p2", "point": "例"}]}]}
        text = ev.render_report(before, after, data, None)
        self.assertIn("只在一边有结果、不计入比较的样例：`chat-02`", text)
        self.assertIn("2 条样例修改前后的结果在同一次判定里判定", text)

    def test_the_spot_check_tells_the_judges_points_from_the_checked_ones(self) -> None:
        before = self.results([[True, False]] * 3, [[True, True]] * 3)
        after = self.results([[True, True]] * 3, [[True, True]] * 3)
        spot = [
            {"id": "prompt-01", "point": "p2", "label": "修改后", "model": "120b", "run": 1, "by": "judge", "judge_kept": True, "my_kept": True},
            {"id": "chat-01", "point": "p1", "label": "修改前", "model": "120b", "run": 2, "by": "rule", "judge_kept": True, "final_kept": False, "my_kept": False, "note": "改口前的说法还在"},
            {"id": "chat-01", "point": "p2", "label": "修改前", "model": "120b", "run": 3, "by": "rule", "judge_kept": True, "final_kept": True, "my_kept": True},
        ]
        text = ev.render_report(before, after, self.DATA, spot)
        self.assertIn("人工抽查了 1 个由判定模型决定的要点，1 个与判定一致", text)
        self.assertIn("另抽查了 2 个机械检查和判定模型共同决定的要点：最终结果 2 个与人工一致，判定模型自己的意见 1 个与人工一致", text)
        self.assertIn("`chat-01` p1（修改前，120b，第 2 次）：判定模型认为保留，人工认为遗漏，最终按机械检查算遗漏。改口前的说法还在", text)

    def test_a_remaining_omission_carries_its_reason_when_one_is_given(self) -> None:
        before = self.results([[True, False]] * 3, [[True, True]] * 3)
        after = self.results([[True, True], [True, False], [True, True]], [[True, True]] * 3)
        text = ev.render_report(before, after, self.DATA, None, {"prompt-01": "候补模型不照规则拆开两条限制"})
        self.assertIn("没有解决的原因：候补模型不照规则拆开两条限制", text)
        self.assertNotIn("没有解决的原因", ev.render_report(before, after, self.DATA, None))

    def test_fallbacks_count_apart_from_omissions(self) -> None:
        t = ev.tally([record("x", "preset:chat", [[False, True], [], [True, True]], statuses=["refined", "truncated", "refined"], purpose=[False, True, True])])
        self.assertEqual((t.runs, t.refined, t.fallback, t.checks, t.omissions, t.purpose), (3, 2, 1, 4, 1, 1))

    def test_the_report_names_the_counts_the_remaining_omissions_and_the_spot_check(self) -> None:
        before = self.results([[True, False]] * 3, [[True, True]] * 3)
        before["meta"]["note"] = "提示词为 f909b335 的版本"
        after = self.results([[True, True], [True, False], [True, True]], [[True, True]] * 3, digest="2", purpose=[True, False, True])
        after["meta"]["ledger"] = {"calls": 1234, "usd": 5.6, "models": {}}
        spot = [
            {"id": "prompt-01", "point": "p2", "label": "修改后", "model": "120b", "run": 2, "judge_kept": False, "my_kept": False},
            {"id": "chat-01", "point": "p1", "label": "修改后", "model": "120b", "run": 1, "judge_kept": True, "my_kept": False, "note": "漏了「晚点」"},
        ]
        text = ev.render_report(before, after, self.DATA, spot)
        self.assertIn("**达成**", text)
        self.assertIn("| 提示词优化（`preset:prompt`） | 改写 | 3 → 1 |", text)
        self.assertIn("| 口语聊天（`preset:chat`） | 未改 | 0 → 0 |", text)
        self.assertIn("提示词为 f909b335 的版本", text)
        self.assertIn("| 全部运行合计（含试跑和改提示词时的试验） | 1234 |", text)
        self.assertIn("### `prompt-01`（提示词优化，openai.gpt-oss-120b-1:0）", text)
        self.assertIn("p2「跳过空文件」：1/3 次遗漏。缺了", text)
        self.assertIn("`prompt-01` 第 2 次（openai.gpt-oss-120b-1:0）：回答了问题", text)
        self.assertIn("人工抽查了 2 个由判定模型决定的要点，1 个与判定一致", text)
        self.assertIn("漏了「晚点」", text)
        self.assertIn(ev.JUDGE_SYSTEM, text)
        self.assertIn("场景的系统提示词包含它使用的预设", text)
        self.assertIn("配对判定的调用记在「修改后」", text)


if __name__ == "__main__":
    unittest.main()
