#!/usr/bin/env python3
"""AI 润色提示词评测（docs/refine-eval.md）。

在 Amazon Bedrock 上用与内置服务相同的模型（openai.gpt-oss-120b-1:0 为主，openai.gpt-oss-20b-1:0
为候补）生成润色结果，逐条判定样例的要点是否保留、结果是否违背预设用途，生成修改前后的对比报告和
提示词摘要记录。请求由 `voltip-refine-eval` 按应用实际发送的内容组装（同样的系统提示词、用户内容和
请求参数），这里只把 `model` 换成 Bedrock 的模型名。

只手动运行（make refine-eval），不进 CI，也不在 make verify 里。每次 AWS 调用前清掉 AWS_* 环境变量，
显式使用 --profile/--region（默认 us / us-west-2）；不打印、不写入任何凭据。

  refine_eval.py generate --out DIR [--models 120b,20b] [--runs 3] [--only ID,...]
  refine_eval.py judge    --out DIR [--pair-with DIR|RESULTS] [--models 120b,20b] [--only ID,...]
                                           --pair-with：另一组提示词的结果和这一组放在同一次判定里，两边都写下
  refine_eval.py recheck  --out DIR        机械检查改过之后，对已判定的结果重新检查（不调用模型）
  refine_eval.py save     --out DIR --to RESULTS [--note TEXT]
  refine_eval.py report   --before RESULTS --after RESULTS [--report FILE] [--spot-check FILE] [--remarks FILE]
  refine_eval.py record   --results RESULTS [--report FILE]
  refine_eval.py all      [--out DIR]      以上全部：上一次提交的结果成为「修改前」，和这一次的结果配对判定
  refine_eval.py calls    DIR...           调用次数、token 和估算花费

DIR 是工作目录（默认 target/refine-eval/latest），RESULTS 是提交进仓库的结果目录
（docs/refine-eval/results/before、…/after）。每次调用都记在 DIR 上一级的 calls.jsonl 里。
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import hashlib
import json
import os
import random
import re
import shutil
import subprocess
import sys
import threading
import time
import unicodedata
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Iterable

REPO = Path(__file__).resolve().parents[2]
SAMPLES = REPO / "scripts/refine-eval/samples.json"
RESULTS = REPO / "docs/refine-eval/results"
REPORT = REPO / "docs/refine-eval/report.md"
RECORD = REPO / "docs/refine-eval/prompt-digests.json"
WORK = REPO / "target/refine-eval/latest"

PROFILE = "us"
REGION = "us-west-2"
# 模型键 → (应用里的模型名, Bedrock 的模型名)。请求按应用里的名字组装：gpt-oss 的推理强度和推理余量
# 由模型名决定（voltip-refine 的 chat_controls）。
MODELS = {
    "120b": ("openai/gpt-oss-120b", "openai.gpt-oss-120b-1:0"),
    "20b": ("openai/gpt-oss-20b", "openai.gpt-oss-20b-1:0"),
}
JUDGE_MODEL = "us.anthropic.claude-opus-5-5"
JUDGE_EFFORT = "medium"
RUNS = 3
# 每次运行允许的调用上限（含重试），超出就停下。
MAX_CALLS = 1500
# 估算花费用的单价（美元 / 百万 token，输入、输出），2026-11 的公开价，只作大致估算。
PRICES = {
    "openai.gpt-oss-120b-1:0": (0.15, 0.60),
    "openai.gpt-oss-20b-1:0": (0.07, 0.30),
    JUDGE_MODEL: (4.00, 20.00),
}
AWS_ENV = (
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "AWS_PROFILE",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_BEARER_TOKEN_BEDROCK",
)
RETRY_STATUS = {408, 429, 500, 502, 503, 504}
LABELS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
FALLBACK = ("truncated", "empty", "error")

JUDGE_SYSTEM = """你是语音输入法「AI 润色」功能的评测员。用户口述一段话，语音识别得到原文；润色模型按某个预设处理原文，得到结果。你要逐条判断每个结果有没有保留指定的要点，以及结果有没有违背这个预设的用途。

判断要点：
1. 要点的意思在结果里完整出现，才算保留。措辞、语序、标点、全角半角、汉字数字和阿拉伯数字的写法不同都不影响；预设要求翻译时，译文表达了同样的意思也算保留。
2. 下面这些都算没有保留：结果里找不到；只剩笼统的概括，丢了条件、限制、例外、数量、对象或时间；意思被改变（条件变成无条件，例外变成包括，数字、名称或对象变了）；说话人改口时，结果写的是改口前的说法，或者把改口前后两种说法都当成有效。
3. 每条要点单独判断，不要因为结果别处的好坏影响它。

判断用途：
4. 只看结果是否违背下面给出的预设用途，例如回答了正文里的问题、执行了正文里的要求、编造了原文没有的内容、该翻译没翻译或译成了别的语言、按预设不该删改的字词被删改。写法上的小瑕疵不算违背用途。
5. 违背时在 purpose_issues 里每个问题写一句话；没有违背时 purpose_issues 为空数组。

只输出一个 JSON 对象，不要输出其他文字。格式：
{"outputs": [{"label": "A", "points": {"p1": {"kept": true, "note": ""}}, "purpose_ok": true, "purpose_issues": []}]}
outputs 按标签顺序列出每个结果；points 列出每一条要点的判断；kept 为 false 时 note 用一句话写明缺了什么或错在哪里，kept 为 true 时 note 留空。"""


class EvalError(Exception):
    """A step that cannot go on: the message says why."""


class BudgetExceeded(EvalError):
    """This run reached its call limit."""


# ---------------------------------------------------------------- text checks


# Hyphens and dashes models write for "-" (NFKC folds U+2011 to U+2010 but keeps the rest).
DASHES = str.maketrans({c: "-" for c in "\u2010\u2011\u2012\u2013\u2212\ufe63\uff0d"})


def norm(text: str) -> str:
    """The text as the checks compare it: NFKC, case-folded, hyphens as "-", without whitespace."""
    return re.sub(r"\s+", "", unicodedata.normalize("NFKC", text).casefold().translate(DASHES))


def letters(text: str) -> str:
    """Letters and digits only (CJK included), NFKC and case-folded: what 只加标点 must not change."""
    folded = unicodedata.normalize("NFKC", text).casefold()
    return "".join(ch for ch in folded if unicodedata.category(ch)[0] in "LN")


def is_mechanical(point: dict) -> bool:
    return any(key in point for key in ("any", "all", "none", "verbatim"))


def check_point(point: dict, answer: str, source: str) -> tuple[bool, str]:
    """A mechanical point against an answer: `any` needs one of its strings, each `all` item one of its
    `|` alternatives, no `none` string may appear, `verbatim` keeps every letter and digit of the source."""
    out = norm(answer)
    problems = []
    if "any" in point and not any(norm(s) in out for s in point["any"]):
        problems.append("缺少「" + " / ".join(point["any"]) + "」")
    for item in point.get("all", []):
        alternatives = item.split("|")
        if not any(norm(a) in out for a in alternatives):
            problems.append("缺少「" + " / ".join(alternatives) + "」")
    for s in point.get("none", []):
        if norm(s) in out:
            problems.append("出现了改口前的「" + s + "」")
    if point.get("verbatim") and letters(answer) != letters(source):
        problems.append("字词有增删或改动")
    return (not problems, "；".join(problems))


def strip_reasoning(content: str) -> tuple[str, int]:
    """Bedrock's gpt-oss puts its reasoning at the start of the content as <reasoning>…</reasoning>; the
    built-in service returns it in a field of its own, so the app's text never holds it. Returns the answer
    and the reasoning's length; an unterminated block leaves no answer."""
    match = re.match(r"\s*<reasoning>(.*?)</reasoning>", content, re.S)
    if match:
        return content[match.end() :], len(match.group(1))
    if content.lstrip().startswith("<reasoning>"):
        return "", len(content)
    return content, 0


def parse_verdict(text: str, labels: list[str], point_ids: list[str]) -> list[dict]:
    """The judge's JSON: one entry per label, in order, each with every point and the purpose."""
    start, end = text.find("{"), text.rfind("}")
    if start < 0 or end < start:
        raise ValueError("no JSON object in the judge's answer")
    data = json.loads(text[start : end + 1])
    outputs = data.get("outputs")
    if not isinstance(outputs, list):
        raise ValueError("no outputs list")
    by_label = {o.get("label"): o for o in outputs if isinstance(o, dict)}
    result = []
    for label in labels:
        entry = by_label.get(label)
        if entry is None:
            raise ValueError(f"no verdict for output {label}")
        points = entry.get("points")
        if not isinstance(points, dict):
            raise ValueError(f"output {label}: no points")
        parsed = {}
        for pid in point_ids:
            verdict = points.get(pid)
            if not isinstance(verdict, dict) or not isinstance(verdict.get("kept"), bool):
                raise ValueError(f"output {label}: no verdict for {pid}")
            parsed[pid] = {"kept": verdict["kept"], "note": str(verdict.get("note") or "")}
        purpose_ok = entry.get("purpose_ok")
        if not isinstance(purpose_ok, bool):
            raise ValueError(f"output {label}: no purpose_ok")
        issues = entry.get("purpose_issues") or []
        if not isinstance(issues, list):
            raise ValueError(f"output {label}: purpose_issues is not a list")
        result.append({"points": parsed, "purpose_ok": purpose_ok, "purpose_issues": [str(i) for i in issues]})
    return result


def sample_sha(sample: dict) -> str:
    """What the judge was shown of a sample: its text and its points' ids and descriptions. The mechanical
    checks are left out, so they can be corrected and run again over stored answers (`recheck`)."""
    shown = {"text": sample["text"], "points": [[p["id"], p["point"]] for p in sample["points"]]}
    return hashlib.sha256(json.dumps(shown, ensure_ascii=False, sort_keys=True).encode("utf-8")).hexdigest()


def canonical(value: Any) -> str:
    """JSON with every object's keys sorted and no spaces: the bytes `voltip-refine-eval` digests
    (canonical_json in crates/voltip-refine-eval)."""
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def request_sha(body: dict) -> str:
    """A request without its model: one sample's requests to both models have the same one."""
    return hashlib.sha256(canonical({k: v for k, v in body.items() if k != "model"}).encode("utf-8")).hexdigest()


def unit_digests(requests: list[dict]) -> dict[str, str]:
    """Each unit's digest over its requests (by sample id: the id, a newline, the request without its model,
    a newline), as `voltip-refine-eval digests` computes it from the code."""
    by_unit: dict[str, list[dict]] = {}
    for r in requests:
        by_unit.setdefault(r["unit"], []).append(r)
    digests = {}
    for unit, rows in sorted(by_unit.items()):
        h = hashlib.sha256()
        for r in sorted(rows, key=lambda r: r["id"]):
            body = {k: v for k, v in r["body"].items() if k != "model"}
            h.update(r["id"].encode("utf-8") + b"\n" + canonical(body).encode("utf-8") + b"\n")
        digests[unit] = h.hexdigest()
    return digests


def changed_units(measured: dict[str, str], current: dict[str, str]) -> list[str]:
    """The units whose digest differs between two sets, or that only one of them has."""
    return sorted(u for u in set(measured) | set(current) if measured.get(u) != current.get(u))


def kept_generations(rows: list[dict], current: dict[str, str]) -> set[tuple[str, int]]:
    """The (sample, run) answers that can be kept: they answered the sample's current request."""
    return {(g["id"], g["run"]) for g in rows if not g.get("error") and g.get("request_sha") is not None and g.get("request_sha") == current.get(g["id"])}


def answers_sha(runs: list[dict]) -> str:
    """The generations a verdict judged: each run's request and raw answer."""
    shown = [[r["run"], r.get("request_sha"), r.get("finish_reason"), r.get("content")] for r in sorted(runs, key=lambda r: r["run"])]
    return hashlib.sha256(json.dumps(shown, ensure_ascii=False).encode("utf-8")).hexdigest()


def is_stale(record: dict | None, sample: dict, answers: str, force: bool, partner: str | None = None) -> bool:
    """A sample is judged again when asked to, when it has no verdict, when its verdict judged other
    answers than `answers` (their answers_sha) or a sample that has since changed what the judge is shown,
    or when it was judged beside other answers than `partner` (the answers_sha of the side it is compared
    with; None: judged alone)."""
    return (
        force
        or record is None
        or record.get("answers_sha") != answers
        or record.get("sample_sha") != sample_sha(sample)
        or record.get("paired_with") != partner
    )


def needs_judging(record: dict | None, sample: dict, runs: list[dict], force: bool, partner: str | None = None) -> bool:
    """[`is_stale`] for a work directory's runs of one sample."""
    return is_stale(record, sample, answers_sha(runs), force, partner)


def check_work(requests: list[dict], generations: list[dict], verdicts: list[dict], samples: list[dict]) -> dict[str, str]:
    """A work directory's results for one model belong together: every answer answered the request on file,
    every sample has a verdict on exactly those answers. Returns the units' digests of those requests."""
    current = {r["id"]: request_sha(r["body"]) for r in requests}
    for g in generations:
        if g.get("request_sha") != current.get(g["id"]):
            raise EvalError(f"{g['id']} run {g['run']}: the answer belongs to another request; run generate again")
    runs: dict[str, list[dict]] = {}
    for g in generations:
        runs.setdefault(g["id"], []).append(g)
    by_id = {v["id"]: v for v in verdicts}
    for sample in samples:
        sid = sample["id"]
        if sid not in current or sid not in runs:
            raise EvalError(f"{sid}: no request or answers; run generate")
        if sid not in by_id:
            raise EvalError(f"{sid}: no verdict; run judge")
        if by_id[sid].get("answers_sha") != answers_sha(runs[sid]):
            raise EvalError(f"{sid}: the verdict judged other answers; run judge again")
        if by_id[sid].get("sample_sha") != sample_sha(sample):
            raise EvalError(f"{sid}: the verdict judged another version of the sample; run judge again")
    return unit_digests([r for r in requests if r["id"] in {s["id"] for s in samples}])


def judge_message(sample: dict, unit: dict, answers: list[str]) -> str:
    """The user message of one judge call: the preset's purpose, the source, the points, the outputs."""
    lines = [
        f"预设：{unit['name']}",
        f"预设用途：{unit['purpose']}",
        "",
        "原文（语音识别结果）：",
        sample["text"],
        "",
        "要点：",
    ]
    lines += [f"- {p['id']}：{p['point']}" for p in sample["points"]]
    lines.append("")
    for label, answer in zip(LABELS, answers):
        lines += [f"结果 {label}：", "<<<", answer, ">>>", ""]
    lines.append("请按系统说明输出 JSON。")
    return "\n".join(lines)


# ---------------------------------------------------------------- samples


def load_samples(path: Path = SAMPLES) -> dict:
    data = json.loads(path.read_text(encoding="utf-8"))
    validate_samples(data)
    return data


def validate_samples(data: dict) -> None:
    """Every sample names a known unit, has unique point ids, a description for each point and well-formed
    checks; every unit has a name and a purpose."""
    units = data.get("units") or {}
    for name, unit in units.items():
        if not unit.get("name") or not unit.get("purpose"):
            raise EvalError(f"unit {name}: name and purpose are required")
    seen = set()
    for sample in data.get("samples", []):
        sid = sample.get("id")
        if sid in seen:
            raise EvalError(f"sample {sid}: id appears twice")
        seen.add(sid)
        if sample.get("unit") not in units:
            raise EvalError(f"sample {sid}: unit {sample.get('unit')!r} has no purpose in units")
        if not str(sample.get("text", "")).strip():
            raise EvalError(f"sample {sid}: no text")
        points = sample.get("points") or []
        if not points:
            raise EvalError(f"sample {sid}: no points")
        ids = [p.get("id") for p in points]
        if len(set(ids)) != len(ids) or not all(ids):
            raise EvalError(f"sample {sid}: point ids must be unique and present")
        for p in points:
            if not p.get("point"):
                raise EvalError(f"sample {sid} {p.get('id')}: no description")
            for key in ("any", "all", "none"):
                if key in p and (not isinstance(p[key], list) or not p[key] or not all(isinstance(s, str) and s.strip() for s in p[key])):
                    raise EvalError(f"sample {sid} {p['id']}: {key} must be a non-empty list of strings")


def unit_order(data: dict) -> list[str]:
    return list((data.get("units") or {}).keys())


# ---------------------------------------------------------------- the eval binary


def eval_tool(*args: str, stdin: str | None = None) -> str:
    """`voltip-refine-eval` from the repository (built by cargo when needed)."""
    env = {k: v for k, v in os.environ.items() if k != "RUSTUP_TOOLCHAIN"}
    cmd = ["cargo", "run", "-q", "--locked", "-p", "voltip-refine-eval", "--", *args]
    done = subprocess.run(cmd, cwd=REPO, env=env, input=stdin, capture_output=True, text=True, check=False)
    if done.returncode != 0:
        raise EvalError(f"voltip-refine-eval {' '.join(args)} failed: {done.stderr.strip()[-2000:]}")
    return done.stdout


def requests_for(app_model: str) -> list[dict]:
    return [json.loads(line) for line in eval_tool("requests", "--model", app_model).splitlines() if line.strip()]


def clean_answers(answers: list[str]) -> list[str]:
    if not answers:
        return []
    return json.loads(eval_tool("clean", stdin=json.dumps(answers, ensure_ascii=False)))


# ---------------------------------------------------------------- ledger


class Ledger:
    """Every Bedrock request, retries included, appended to calls.jsonl; a run stops at `max_calls`."""

    def __init__(self, path: Path, max_calls: int, tag: str) -> None:
        self.path, self.max_calls, self.tag = path, max_calls, tag
        self.lock = threading.Lock()
        self.used = 0
        path.parent.mkdir(parents=True, exist_ok=True)

    def reserve(self) -> None:
        with self.lock:
            if self.used >= self.max_calls:
                raise BudgetExceeded(f"this run reached its limit of {self.max_calls} calls (--max-calls)")
            self.used += 1

    def record(self, kind: str, model: str, status: str, input_tokens: int = 0, output_tokens: int = 0) -> None:
        entry = {
            "ts": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
            "dir": self.tag,
            "kind": kind,
            "model": model,
            "status": status,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
        }
        with self.lock, self.path.open("a", encoding="utf-8") as f:
            f.write(json.dumps(entry, ensure_ascii=False) + "\n")


def summarize_calls(entries: Iterable[dict]) -> dict:
    """Calls, tokens and the estimated cost, per model and in total."""
    per_model: dict[str, dict] = {}
    for e in entries:
        m = per_model.setdefault(e["model"], {"calls": 0, "failed": 0, "input_tokens": 0, "output_tokens": 0})
        m["calls"] += 1
        m["failed"] += 0 if e["status"] == "ok" else 1
        m["input_tokens"] += e.get("input_tokens", 0)
        m["output_tokens"] += e.get("output_tokens", 0)
    total_cost = 0.0
    for model, m in per_model.items():
        price_in, price_out = PRICES.get(model, (0.0, 0.0))
        m["usd"] = round(m["input_tokens"] / 1e6 * price_in + m["output_tokens"] / 1e6 * price_out, 4)
        total_cost += m["usd"]
    return {"models": per_model, "calls": sum(m["calls"] for m in per_model.values()), "usd": round(total_cost, 4)}


def read_jsonl(path: Path) -> list[dict]:
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def write_jsonl(path: Path, rows: Iterable[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    with tmp.open("w", encoding="utf-8") as f:
        for row in rows:
            f.write(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n")
    tmp.replace(path)


# ---------------------------------------------------------------- Bedrock


def drop_aws_env() -> None:
    """Credentials and regions in the environment outrank the profile: never let them in."""
    for name in AWS_ENV:
        os.environ.pop(name, None)


@dataclass
class Bedrock:
    """Clients for one run: SigV4-signed OpenAI chat completions and the Converse judge."""

    ledger: Ledger
    profile: str = PROFILE
    region: str = REGION
    _boto: Any = field(default=None, init=False)
    _converse: Any = field(default=None, init=False)

    def __post_init__(self) -> None:
        drop_aws_env()
        import boto3  # noqa: PLC0415 - only the network commands need it
        from botocore.config import Config  # noqa: PLC0415

        self._boto = boto3.Session(profile_name=self.profile, region_name=self.region)
        if self._boto.get_credentials() is None:
            raise EvalError(f"no credentials for profile {self.profile}")
        self._converse = self._boto.client(
            "bedrock-runtime", region_name=self.region, config=Config(retries={"total_max_attempts": 1}, read_timeout=600, connect_timeout=30)
        )

    def chat(self, model: str, body: dict) -> dict:
        """POST /openai/v1/chat/completions with retries; the parsed answer."""
        import urllib.error  # noqa: PLC0415
        import urllib.request  # noqa: PLC0415

        from botocore.auth import SigV4Auth  # noqa: PLC0415
        from botocore.awsrequest import AWSRequest  # noqa: PLC0415

        url = f"https://bedrock-runtime.{self.region}.amazonaws.com/openai/v1/chat/completions"
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        last = "no attempt"
        for attempt in range(6):
            self.ledger.reserve()
            creds = self._boto.get_credentials().get_frozen_credentials()
            signed = AWSRequest(method="POST", url=url, data=data, headers={"Content-Type": "application/json"})
            SigV4Auth(creds, "bedrock", self.region).add_auth(signed)
            request = urllib.request.Request(url, data=data, headers=dict(signed.headers.items()), method="POST")
            try:
                with urllib.request.urlopen(request, timeout=180) as response:
                    answer = json.load(response)
                usage = answer.get("usage") or {}
                self.ledger.record("generate", model, "ok", usage.get("prompt_tokens", 0), usage.get("completion_tokens", 0))
                return answer
            except urllib.error.HTTPError as e:
                text = e.read().decode("utf-8", "replace")[:300]
                self.ledger.record("generate", model, f"http {e.code}")
                last = f"HTTP {e.code}: {text}"
                if e.code not in RETRY_STATUS:
                    raise EvalError(last) from e
            except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
                self.ledger.record("generate", model, "network")
                last = f"network: {e}"
            time.sleep(min(60, 2**attempt + random.random()))
        raise EvalError(f"{model}: gave up after retries ({last})")

    def judge(self, system: str, message: str) -> tuple[str, int, int]:
        """One Converse call to the judge; the answer's text and its token counts."""
        from botocore.exceptions import ClientError, ConnectionError as BotoConnectionError, ReadTimeoutError  # noqa: PLC0415

        retry_codes = {"ThrottlingException", "ServiceUnavailableException", "InternalServerException", "ModelNotReadyException", "ModelErrorException"}
        last = "no attempt"
        for attempt in range(6):
            self.ledger.reserve()
            try:
                response = self._converse.converse(
                    modelId=JUDGE_MODEL,
                    system=[{"text": system}],
                    messages=[{"role": "user", "content": [{"text": message}]}],
                    inferenceConfig={"maxTokens": 16000},
                    additionalModelRequestFields={"output_config": {"effort": JUDGE_EFFORT}},
                )
                usage = response.get("usage") or {}
                self.ledger.record("judge", JUDGE_MODEL, "ok", usage.get("inputTokens", 0), usage.get("outputTokens", 0))
                blocks = response["output"]["message"]["content"]
                text = "".join(b["text"] for b in blocks if "text" in b)
                return text, usage.get("inputTokens", 0), usage.get("outputTokens", 0)
            except ClientError as e:
                code = e.response.get("Error", {}).get("Code", "")
                self.ledger.record("judge", JUDGE_MODEL, code or "error")
                last = f"{code}: {e}"
                if code not in retry_codes:
                    raise EvalError(last) from e
            except (BotoConnectionError, ReadTimeoutError) as e:
                self.ledger.record("judge", JUDGE_MODEL, "network")
                last = f"network: {e}"
            time.sleep(min(60, 2**attempt + random.random()))
        raise EvalError(f"judge: gave up after retries ({last})")


class Appender:
    """Appends result rows to a JSON Lines file as they arrive, so an interrupted run keeps its work."""

    def __init__(self, path: Path) -> None:
        self.path, self.lock = path, threading.Lock()
        path.parent.mkdir(parents=True, exist_ok=True)

    def __call__(self, row: dict) -> None:
        with self.lock, self.path.open("a", encoding="utf-8") as f:
            f.write(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n")


def parallel(items: list, worker: Callable, workers: int, sink: Callable[[dict], None]) -> int:
    """Run `worker` over `items` on threads, handing each result to `sink`; a budget stop or an error
    cancels what has not started and is raised. Returns how many results arrived."""
    count = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        futures = [pool.submit(worker, item) for item in items]
        try:
            for future in concurrent.futures.as_completed(futures):
                sink(future.result())
                count += 1
        except BaseException:
            for f in futures:
                f.cancel()
            raise
    return count


def compact(path: Path, key: tuple[str, ...]) -> list[dict]:
    """The file's rows with the last one of each key kept, sorted, written back."""
    rows = merge_rows([], read_jsonl(path), key)
    write_jsonl(path, rows)
    return rows


# ---------------------------------------------------------------- generate


def generation_status(answer: dict, cleaned: str | None) -> str:
    if answer.get("error"):
        return "error"
    if answer.get("finish_reason") == "length":
        return "truncated"
    return "refined" if cleaned else "empty"


def cmd_generate(args: argparse.Namespace) -> None:
    data = load_samples()
    out = Path(args.out)
    ledger = Ledger(out.parent / "calls.jsonl", args.max_calls, out.name)
    bedrock = Bedrock(ledger, args.profile, args.region)
    only = set(filter(None, (args.only or "").split(",")))
    known = {s["id"] for s in data["samples"]}
    if only - known:
        raise EvalError(f"unknown samples: {sorted(only - known)}")
    meta_path = out / "meta.json"
    meta = json.loads(meta_path.read_text()) if meta_path.exists() else {}
    meta.update({"commit": git_commit(), "generated_at": dt.date.today().isoformat(), "runs": args.runs, "lengths": json.loads(eval_tool("lengths"))})
    out.mkdir(parents=True, exist_ok=True)
    meta_path.write_text(json.dumps(meta, ensure_ascii=False, indent=1, sort_keys=True) + "\n")
    for key in args.models.split(","):
        app_model, bedrock_model = MODELS[key]
        requests = [r for r in requests_for(app_model) if not only or r["id"] in only]
        write_jsonl(out / f"requests-{key}.jsonl", merge_rows(read_jsonl(out / f"requests-{key}.jsonl"), requests, ("id",)))
        path = out / f"generations-{key}.jsonl"
        current = {r["id"]: request_sha(r["body"]) for r in requests}
        done = kept_generations(read_jsonl(path), current)
        todo = [(r, run) for r in requests for run in range(1, args.runs + 1) if (r["id"], run) not in done]
        print(f"[generate] {key}: {len(todo)} calls to make, {len(done)} kept", file=sys.stderr)

        def one(item: tuple[dict, int], bedrock_model: str = bedrock_model) -> dict:
            request, run = item
            body = dict(request["body"], model=bedrock_model)
            started = time.monotonic()
            try:
                answer = bedrock.chat(bedrock_model, body)
            except BudgetExceeded:
                raise
            except EvalError as e:
                return {"id": request["id"], "unit": request["unit"], "run": run, "request_sha": current[request["id"]], "error": str(e)}
            choice = (answer.get("choices") or [{}])[0]
            message = choice.get("message") or {}
            content, reasoning = strip_reasoning(message.get("content") or "")
            usage = answer.get("usage") or {}
            return {
                "id": request["id"],
                "unit": request["unit"],
                "run": run,
                "request_sha": current[request["id"]],
                "finish_reason": choice.get("finish_reason"),
                "content": content,
                "reasoning_chars": reasoning,
                "prompt_tokens": usage.get("prompt_tokens", 0),
                "completion_tokens": usage.get("completion_tokens", 0),
                "latency_ms": int((time.monotonic() - started) * 1000),
            }

        try:
            parallel(todo, one, args.workers, Appender(path))
        finally:
            rows = compact(path, ("id", "run"))
        errors = [g for g in rows if g.get("error")]
        print(f"[generate] {key}: {len(rows)} answers, {len(errors)} errors; {ledger.used} calls this run", file=sys.stderr)


def merge_rows(old: list[dict], new: list[dict], key: tuple[str, ...]) -> list[dict]:
    """`old` and `new` by `key`, the later row winning, sorted by key."""
    merged = {}
    for row in [*old, *new]:
        merged[tuple(row[k] for k in key)] = row
    return [merged[k] for k in sorted(merged)]


def git_commit() -> str:
    done = subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO, capture_output=True, text=True, check=False)
    dirty = subprocess.run(["git", "status", "--porcelain", "--", "crates", "scripts/refine-eval"], cwd=REPO, capture_output=True, text=True, check=False)
    return done.stdout.strip() + ("+dirty" if dirty.stdout.strip() else "")


# ---------------------------------------------------------------- judge


JUDGE_MISSED = "判定模型认为没有保留（机械检查通过）"


def decide_point(point: dict, answer: str, source: str, judged: dict | None) -> dict:
    """One point of one answer. A string check (any / all / none) and the judge must both find it kept:
    the check catches a figure that is missing or a value the speaker took back, the judge one that is
    there but changed or attached to the wrong thing. A verbatim check decides alone (a capital letter is
    the purpose's business, not a dropped point); a point without a check is the judge's."""
    if not is_mechanical(point):
        return {"kept": judged["kept"], "by": "judge", "note": judged["note"]} if judged else {"kept": None, "by": "judge", "note": "未判定"}
    rule, note = check_point(point, answer, source)
    opinion = judged["kept"] if judged else None
    entry = {"kept": rule, "by": "rule", "note": note, "rule": rule, "judge": opinion, "judge_note": judged["note"] if judged else ""}
    if rule and opinion is False and not point.get("verbatim"):
        entry.update(kept=False, by="judge", note=entry["judge_note"] or JUDGE_MISSED)
    return entry


def evaluate_runs(sample: dict, runs: list[dict], verdicts: list[dict] | None) -> list[dict]:
    """Each run's final verdict on every point ([`decide_point`]) and the judge's on the purpose. Runs that
    fell back are not judged."""
    out = []
    refined = [r for r in runs if r["status"] == "refined"]
    for run in runs:
        entry = {k: run[k] for k in ("run", "status", "answer", "finish_reason", "prompt_tokens", "completion_tokens", "reasoning_chars") if k in run}
        if run["status"] != "refined":
            out.append(entry)
            continue
        verdict = verdicts[refined.index(run)] if verdicts else None
        points = {}
        for p in sample["points"]:
            points[p["id"]] = decide_point(p, run["answer"], sample["text"], verdict["points"][p["id"]] if verdict else None)
        entry["points"] = points
        entry["purpose_ok"] = verdict["purpose_ok"] if verdict else None
        entry["purpose_issues"] = verdict["purpose_issues"] if verdict else []
        out.append(entry)
    return out


def judge_sample(judge: Any, sample: dict, unit: dict, run_sets: list[list[dict]], seed: str) -> tuple[list[list[dict] | None], dict]:
    """One judge call for every refined answer of one sample in `run_sets` (this run's, and the other
    side's when two runs are compared): each distinct answer once, in an order shuffled by `seed`, the
    judge not told which run or side an answer came from. Returns each set's verdicts in the order of its
    refined runs (None when it has none) and the call's usage."""
    usage = {"calls": 0, "input_tokens": 0, "output_tokens": 0}
    answers = sorted({r["answer"] for runs in run_sets for r in runs if r["status"] == "refined"})
    if not answers:
        return [None for _ in run_sets], usage
    random.Random(seed).shuffle(answers)
    labels = list(LABELS[: len(answers)])
    message = judge_message(sample, unit, answers)
    point_ids = [p["id"] for p in sample["points"]]
    verdicts = None
    for _ in range(3):
        text, used_in, used_out = judge.judge(JUDGE_SYSTEM, message)
        usage = {"calls": usage["calls"] + 1, "input_tokens": usage["input_tokens"] + used_in, "output_tokens": usage["output_tokens"] + used_out}
        try:
            verdicts = parse_verdict(text, labels, point_ids)
            break
        except (ValueError, json.JSONDecodeError) as e:
            print(f"[judge] {sample['id']}: unusable answer ({e}), asking again", file=sys.stderr)
    if verdicts is None:
        raise EvalError(f"{sample['id']}: the judge gave no usable verdict in 3 tries")
    by_answer = dict(zip(answers, verdicts))
    return [[by_answer[r["answer"]] for r in runs if r["status"] == "refined"] or None for runs in run_sets], usage


def load_runs(out: Path, key: str) -> dict[str, list[dict]]:
    """A work directory's answers by sample, cleaned as the app cleans them, each with its status."""
    generations = read_jsonl(out / f"generations-{key}.jsonl")
    if not generations:
        raise EvalError(f"no generations for {key} in {out}: run generate first")
    errors = [g for g in generations if g.get("error")]
    if errors:
        raise EvalError(f"{key}: {len(errors)} generations in {out} failed; run generate again to fill them in")
    runs_by_id: dict[str, list[dict]] = {}
    for g, answer in zip(generations, clean_answers([g.get("content", "") for g in generations])):
        g = dict(g, answer=answer)
        g["status"] = generation_status(g, answer)
        runs_by_id.setdefault(g["id"], []).append(g)
    for runs in runs_by_id.values():
        runs.sort(key=lambda r: r["run"])
    return runs_by_id


@dataclass
class Side:
    """One set of answers to judge, by sample: their runs, their answers_sha, the verdicts on file and
    where verdicts are written."""

    runs: dict[str, list[dict]]
    shas: dict[str, str]
    existing: dict[str, dict]
    path: Path


def work_side(out: Path, key: str) -> Side:
    runs = load_runs(out, key)
    path = out / f"verdicts-{key}.jsonl"
    return Side(runs, {sid: answers_sha(r) for sid, r in runs.items()}, {v["id"]: v for v in read_jsonl(path)}, path)


def results_side(results: Path, key: str) -> Side:
    """A saved results directory (`save`): its records keep the answers and the answers_sha computed when
    they were judged, so they can be judged again beside other answers without their generations."""
    path = results / f"{key}.jsonl"
    records = {r["id"]: r for r in read_jsonl(path)}
    if not records:
        raise EvalError(f"no results for {key} in {results}")
    return Side({sid: r["runs"] for sid, r in records.items()}, {sid: r["answers_sha"] for sid, r in records.items()}, records, path)


def side_of(path: Path, key: str) -> Side:
    """A work directory when it has generations, otherwise a saved results directory."""
    return work_side(path, key) if (path / f"generations-{key}.jsonl").exists() else results_side(path, key)


def cmd_judge(args: argparse.Namespace) -> None:
    """Judge a work directory's answers. With --pair-with (another work directory, or a saved results
    directory), each sample's answers there are judged in the same call, and the verdicts of both are
    written, each beside its answers. A sample the other side lacks is judged alone."""
    data = load_samples()
    by_id = {s["id"]: s for s in data["samples"]}
    out = Path(args.out)
    pair = Path(args.pair_with) if getattr(args, "pair_with", None) else None
    ledger = Ledger(out.parent / "calls.jsonl", args.max_calls, out.name)
    bedrock = Bedrock(ledger, args.profile, args.region)
    only = set(filter(None, (args.only or "").split(",")))
    for key in args.models.split(","):
        mine = work_side(out, key)
        other = side_of(pair, key) if pair else None

        def partner(sid: str, other: Side | None = other) -> Side | None:
            return other if other is not None and sid in other.runs else None

        def stale(sid: str, mine: Side = mine) -> bool:
            theirs = partner(sid)
            if theirs is None:
                return is_stale(mine.existing.get(sid), by_id[sid], mine.shas[sid], args.force)
            return is_stale(mine.existing.get(sid), by_id[sid], mine.shas[sid], args.force, theirs.shas[sid]) or is_stale(
                theirs.existing.get(sid), by_id[sid], theirs.shas[sid], args.force, mine.shas[sid]
            )

        todo = [sid for sid in mine.runs if (not only or sid in only) and stale(sid)]
        print(f"[judge] {key}: {len(todo)} samples to judge" + (f" beside {pair}" if pair else ""), file=sys.stderr)

        def one(sid: str, mine: Side = mine) -> list[dict | None]:
            sample = by_id[sid]
            theirs = partner(sid)
            sides = [mine] + ([theirs] if theirs else [])
            verdicts, usage = judge_sample(bedrock, sample, data["units"][sample["unit"]], [s.runs[sid] for s in sides], f"{sid}:{key}")
            rows: list[dict | None] = []
            for i, side in enumerate(sides):
                rows.append(
                    {
                        "id": sid,
                        "unit": sample["unit"],
                        "sample_sha": sample_sha(sample),
                        "answers_sha": side.shas[sid],
                        "paired_with": sides[1 - i].shas[sid] if theirs else None,
                        "runs": evaluate_runs(sample, side.runs[sid], verdicts[i]),
                        "judge": usage if i == 0 else {"calls": 0, "input_tokens": 0, "output_tokens": 0},
                    }
                )
            return rows + [None] * (2 - len(rows))

        mine_sink = Appender(mine.path)
        other_sink = Appender(other.path) if other is not None else None

        def sink(rows: list[dict | None]) -> None:
            mine_sink(rows[0])
            if other_sink and rows[1] is not None:
                other_sink(rows[1])

        try:
            parallel(todo, one, args.workers, sink)
        finally:
            judged = compact(mine.path, ("id",))
            if other is not None:
                compact(other.path, ("id",))
        print(f"[judge] {key}: {len(judged)} samples judged; {ledger.used} calls this run", file=sys.stderr)


def recorded_opinion(entry: dict) -> dict | None:
    """The judge's verdict on a point as a record holds it: beside a check's result, or as the point's own
    (records made before the judge's note was kept beside a check have none)."""
    if entry.get("by") == "rule" or "rule" in entry or "judge" in entry:
        return None if entry.get("judge") is None else {"kept": entry["judge"], "note": entry.get("judge_note", "")}
    return {"kept": entry["kept"], "note": entry.get("note", "")}


def recheck_record(record: dict, sample: dict) -> dict:
    """A judged record with its points decided again ([`decide_point`]) against the current samples' checks,
    from the stored answers and the judge's recorded verdicts. Refused when the judge was shown a different
    text or other points."""
    expected = sample_sha(sample)
    if record.get("sample_sha", expected) != expected:
        raise EvalError(f"{record['id']}: the sample's text or points changed since it was judged; run judge --force")
    runs = []
    for run in record["runs"]:
        run = dict(run)
        if run["status"] == "refined":
            run["points"] = {p["id"]: decide_point(p, run["answer"], sample["text"], recorded_opinion(run["points"][p["id"]])) for p in sample["points"]}
        runs.append(run)
    return {**record, "sample_sha": expected, "runs": runs}


def cmd_recheck(args: argparse.Namespace) -> None:
    data = load_samples()
    by_id = {s["id"]: s for s in data["samples"]}
    out = Path(args.out)
    for key in args.models.split(","):
        path = out / f"verdicts-{key}.jsonl"
        records = read_jsonl(path)
        write_jsonl(path, [recheck_record(r, by_id[r["id"]]) for r in records])
        print(f"[recheck] {key}: {len(records)} samples checked again", file=sys.stderr)


# ---------------------------------------------------------------- save, report, record


def cmd_save(args: argparse.Namespace) -> None:
    """Copy a work directory's verdicts, prompts and call summary into a results directory."""
    out, to = Path(args.out), Path(args.to)
    meta = json.loads((out / "meta.json").read_text())
    to.mkdir(parents=True, exist_ok=True)
    prompts: dict[str, dict] = {}
    samples = load_samples()["samples"]
    measured: dict[str, dict[str, str]] = {}
    for key in MODELS:
        verdicts = read_jsonl(out / f"verdicts-{key}.jsonl")
        if not verdicts:
            (to / f"{key}.jsonl").unlink(missing_ok=True)
            continue
        requests = read_jsonl(out / f"requests-{key}.jsonl")
        measured[key] = check_work(requests, read_jsonl(out / f"generations-{key}.jsonl"), verdicts, samples)
        write_jsonl(to / f"{key}.jsonl", verdicts)
        for request in requests:
            system = request["body"]["messages"][0]["content"]
            digest = hashlib.sha256(system.encode("utf-8")).hexdigest()
            prompts.setdefault(request["unit"], {})[digest] = system
        meta.setdefault("models", {})[key] = MODELS[key][1]
    digests = list(measured.values())
    if not digests:
        raise EvalError(f"{out}: no verdicts to save")
    if any(changed_units(digests[0], d) for d in digests[1:]):
        raise EvalError(f"{out}: the models were sent different requests")
    meta["digests"] = digests[0]
    calls = [c for c in read_jsonl(out.parent / "calls.jsonl") if c.get("dir") == out.name]
    meta["calls"] = summarize_calls(calls)
    meta["ledger"] = summarize_calls(read_jsonl(out.parent / "calls.jsonl"))
    if getattr(args, "note", None):
        meta["note"] = args.note
    meta["judge"] = {"model": JUDGE_MODEL, "effort": JUDGE_EFFORT}
    meta["request_params"] = request_params(out)
    (to / "meta.json").write_text(json.dumps(meta, ensure_ascii=False, indent=1, sort_keys=True) + "\n")
    rows = {unit: [{"sha256": d, "system": s} for d, s in sorted(v.items())] for unit, v in sorted(prompts.items())}
    (to / "prompts.json").write_text(json.dumps(rows, ensure_ascii=False, indent=1) + "\n")
    print(f"[save] {out} → {to}", file=sys.stderr)


def request_params(out: Path) -> dict:
    """The request fields besides the messages, as the app sent them (one entry per distinct set)."""
    seen = set()
    for key in MODELS:
        for request in read_jsonl(out / f"requests-{key}.jsonl"):
            params = {k: v for k, v in request["body"].items() if k not in ("messages", "max_tokens", "model")}
            seen.add(json.dumps(params, sort_keys=True))
    return {"fields": [json.loads(s) for s in sorted(seen)], "max_tokens": "min(按预设的输出预算 + 256, max(900, 输出预算))"}


@dataclass
class Tally:
    samples: int = 0
    runs: int = 0
    refined: int = 0
    fallback: int = 0
    checks: int = 0
    omissions: int = 0
    purpose: int = 0
    rule_points: int = 0
    rule_agree: int = 0

    def add(self, other: "Tally") -> None:
        for name in self.__dataclass_fields__:
            setattr(self, name, getattr(self, name) + getattr(other, name))


def tally(records: list[dict]) -> Tally:
    t = Tally()
    for record in records:
        t.samples += 1
        for run in record["runs"]:
            t.runs += 1
            if run["status"] != "refined":
                t.fallback += 1
                continue
            t.refined += 1
            for verdict in run["points"].values():
                t.checks += 1
                t.omissions += 0 if verdict["kept"] else 1
                if "rule" in verdict and verdict.get("judge") is not None:
                    t.rule_points += 1
                    t.rule_agree += 1 if verdict["judge"] == verdict["rule"] else 0
            t.purpose += 0 if run.get("purpose_ok") is not False else 1
    return t


def load_results(path: Path) -> dict:
    results = {"meta": json.loads((path / "meta.json").read_text()), "models": {}}
    for key in MODELS:
        records = read_jsonl(path / f"{key}.jsonl")
        if records:
            results["models"][key] = {r["id"]: r for r in records}
    prompts = path / "prompts.json"
    results["prompts"] = json.loads(prompts.read_text()) if prompts.exists() else {}
    return results


def by_unit(records: dict) -> dict[str, list[dict]]:
    out: dict[str, list[dict]] = {}
    for record in records.values():
        out.setdefault(record["unit"], []).append(record)
    return out


def compare(before: dict, after: dict, units: list[str], key: str) -> dict:
    """Per unit and in total: omissions, purpose violations and fallbacks before and after, and whether the
    goal holds (no unit has more omissions, the total is lower)."""
    rows, total_before, total_after = [], Tally(), Tally()
    b_records, a_records = before["models"].get(key, {}), after["models"].get(key, {})
    both = set(b_records) & set(a_records)
    b_units = by_unit({sid: r for sid, r in b_records.items() if sid in both})
    a_units = by_unit({sid: r for sid, r in a_records.items() if sid in both})
    for unit in units:
        tb, ta = tally(b_units.get(unit, [])), tally(a_units.get(unit, []))
        total_before.add(tb)
        total_after.add(ta)
        rows.append({"unit": unit, "before": tb, "after": ta, "ok": ta.omissions <= tb.omissions})
    goal = all(r["ok"] for r in rows) and total_after.omissions < total_before.omissions
    return {"rows": rows, "before": total_before, "after": total_after, "goal": goal}


def sample_omissions(record: dict, sample: dict) -> list[dict]:
    """The points a sample's answers dropped: which, how many runs out of how many, and why."""
    out = []
    refined = [r for r in record["runs"] if r["status"] == "refined"]
    for p in sample["points"]:
        missed = [r for r in refined if not r["points"][p["id"]]["kept"]]
        if missed:
            out.append({"point": p, "missed": len(missed), "of": len(refined), "note": missed[0]["points"][p["id"]]["note"], "answer": missed[0]["answer"]})
    return out


def fmt_delta(before: int, after: int) -> str:
    return f"{before} → {after}"


def render_report(before: dict, after: dict, data: dict, spot: list[dict] | None, remarks: dict[str, str] | None = None) -> str:
    units = unit_order(data)
    names = {u: data["units"][u]["name"] for u in units}
    samples = {s["id"]: s for s in data["samples"]}
    keys = [k for k in MODELS if k in before["models"] and k in after["models"]]
    lines = ["# AI 润色提示词评测报告", ""]
    lines.append("由 `scripts/refine-eval/refine_eval.py report` 生成，方法和用法见 [docs/refine-eval.md](../refine-eval.md)。")
    lines.append("")
    bm, am = before["meta"], after["meta"]
    lines += [
        f"- 修改前：提交 `{bm.get('commit', '?')}`，{bm.get('generated_at', '?')} 运行" + (f"（{bm['note']}）" if bm.get("note") else ""),
        f"- 修改后：提交 `{am.get('commit', '?')}`，{am.get('generated_at', '?')} 运行" + (f"（{am['note']}）" if am.get("note") else ""),
        f"- 样例：{len(data['samples'])} 条，{sum(len(s['points']) for s in data['samples'])} 个要点；每条样例每个模型生成 {am.get('runs', RUNS)} 次",
        "- 生成模型：" + "、".join(f"`{MODELS[k][1]}`（应用里的 `{MODELS[k][0]}`）" for k in keys),
        f"- 判定模型：`{JUDGE_MODEL}`，effort `{JUDGE_EFFORT}`",
        "",
        "「提示词」一栏比较这个预设或场景修改前后发出的请求：「改写」是系统提示词或请求参数变了，「未改」是请求逐字相同；"
        "场景的系统提示词包含它使用的预设，预设改了，场景也是「改写」（场景要求本身的长度见「提示词长度」）。"
        "「遗漏」是润色结果没有保留的要点次数（每次生成、每个要点算一次）；「违背用途」是判定模型认为结果违背预设用途的生成次数；"
        "「回退」是应用会改用原文的生成次数（输出被 `max_tokens` 截断或清理后为空），这些生成不计入遗漏和违背用途。",
        "",
    ]
    for key in keys:
        result = compare(before, after, units, key)
        lines += [f"## {MODELS[key][1]}", ""]
        verdict = "达成" if result["goal"] else "未达成"
        lines.append(f"目标（每个预设和场景的遗漏都不多于修改前，总数下降）：**{verdict}**。")
        lines.append("")
        lines.append("| 预设或场景 | 提示词 | 遗漏 | 遗漏率 | 违背用途 | 回退 |")
        lines.append("| --- | --- | --- | --- | --- | --- |")
        b_digests, a_digests = bm.get("digests") or {}, am.get("digests") or {}
        for row in result["rows"] + [{"unit": None, "before": result["before"], "after": result["after"], "ok": result["goal"]}]:
            tb, ta = row["before"], row["after"]
            unit = row["unit"]
            label = "合计" if unit is None else f"{names[unit]}（`{unit}`）"
            if unit is None:
                prompt = f"{len(changed_units(b_digests, a_digests))} 个改写"
            elif unit in b_digests and unit in a_digests:
                prompt = "未改" if b_digests[unit] == a_digests[unit] else "改写"
            else:
                prompt = "?"
            mark = "" if row["ok"] else " ⚠"
            rate = f"{pct(tb.omissions, tb.checks)} → {pct(ta.omissions, ta.checks)}"
            lines.append(f"| {label} | {prompt} | {fmt_delta(tb.omissions, ta.omissions)}{mark} | {rate} | {fmt_delta(tb.purpose, ta.purpose)} | {fmt_delta(tb.fallback, ta.fallback)} |")
        b_records, a_records = before["models"][key], after["models"][key]
        one_side = sorted(set(b_records) ^ set(a_records))
        if one_side:
            lines += ["", "只在一边有结果、不计入比较的样例：" + "、".join(f"`{sid}`" for sid in one_side) + "。"]
        paired = [sid for sid in a_records if sid in b_records and a_records[sid].get("paired_with") is not None and a_records[sid]["paired_with"] == b_records[sid].get("answers_sha")]
        both = len(set(b_records) & set(a_records))
        lines += [
            "",
            f"{len(paired)} 条样例修改前后的结果在同一次判定里判定（共 {both} 条）：相同的结果只判定一次，顺序打乱，判定模型不知道结果来自哪一版。",
        ]
        t = result["after"]
        if t.rule_points:
            lines += ["", f"判定模型与机械检查在 {t.rule_points} 个机械要点上的一致率（修改后）：{pct(t.rule_agree, t.rule_points)}。"]
        lines.append("")
    lines += ["## 仍有遗漏的样例（修改后）", ""]
    any_left = False
    for key in keys:
        records = after["models"][key]
        for sid in sorted(records, key=lambda s: (units.index(records[s]["unit"]), s)):
            dropped = sample_omissions(records[sid], samples[sid])
            if not dropped:
                continue
            any_left = True
            lines.append(f"### `{sid}`（{names[records[sid]['unit']]}，{MODELS[key][1]}）")
            lines += ["", f"原文：{samples[sid]['text']}", ""]
            for d in dropped:
                lines.append(f"- {d['point']['id']}「{d['point']['point']}」：{d['missed']}/{d['of']} 次遗漏。{d['note']}")
            lines += ["", "其中一次的结果：", "", "```text", dropped[0]["answer"], "```", ""]
            if remarks and remarks.get(sid):
                lines += [f"没有解决的原因：{remarks[sid]}", ""]
    if not any_left:
        lines += ["没有。", ""]
    lines += ["## 违背用途的结果（修改后）", ""]
    issues = []
    for key in keys:
        for sid, record in sorted(after["models"][key].items()):
            for run in record["runs"]:
                if run.get("purpose_ok") is False:
                    issues.append(f"- `{sid}` 第 {run['run']} 次（{MODELS[key][1]}）：" + "；".join(run.get("purpose_issues") or ["（未写明）"]))
    lines += (issues or ["没有。"]) + [""]
    lines += ["## 提示词长度", ""]
    lines.append("预设的长度是预设本身的文字（复制为自定义预设时复制的部分，上限 4000 字）；场景是场景要求（上限 500 字）。")
    lines.append("")
    lines.append("| 预设或场景 | 修改前 | 修改后 | 上限 | 系统提示词平均 token（修改前 → 修改后） |")
    lines.append("| --- | --- | --- | --- | --- |")
    b_len = {r["unit"]: r for r in bm.get("lengths", [])}
    a_len = {r["unit"]: r for r in am.get("lengths", [])}
    first = keys[0] if keys else None
    for unit in units:
        b, a = b_len.get(unit, {}), a_len.get(unit, {})
        tokens = ""
        if first:
            tokens = f"{avg_prompt_tokens(before['models'][first], unit)} → {avg_prompt_tokens(after['models'][first], unit)}"
        lines.append(f"| {names[unit]} | {b.get('chars', '?')} | {a.get('chars', '?')} | {a.get('limit', b.get('limit', '?'))} | {tokens} |")
    lines += ["", "## 判定模型抽查", ""]
    if spot:
        word = {True: "保留", False: "遗漏"}
        judged = [s for s in spot if s.get("by", "judge") == "judge"]
        checked = [s for s in spot if s.get("by", "judge") == "rule"]
        if judged:
            agree = sum(1 for s in judged if s["judge_kept"] == s["my_kept"])
            lines.append(f"人工抽查了 {len(judged)} 个由判定模型决定的要点，{agree} 个与判定一致。不一致的：")
            lines.append("")
            lines += [
                f"- `{s['id']}` {s['point']}（{s['label']}，{s['model']}，第 {s['run']} 次）：判定{word[s['judge_kept']]}，人工{word[s['my_kept']]}。{s.get('note', '')}"
                for s in judged
                if s["judge_kept"] != s["my_kept"]
            ] or ["（没有。）"]
        if checked:
            final = sum(1 for s in checked if s["final_kept"] == s["my_kept"])
            own = sum(1 for s in checked if s["judge_kept"] == s["my_kept"])
            lines += [
                "",
                f"另抽查了 {len(checked)} 个机械检查和判定模型共同决定的要点：最终结果 {final} 个与人工一致，判定模型自己的意见 {own} 个与人工一致。不一致的：",
                "",
            ]
            lines += [
                f"- `{s['id']}` {s['point']}（{s['label']}，{s['model']}，第 {s['run']} 次）：判定模型认为{word[s['judge_kept']]}，人工认为{word[s['my_kept']]}，"
                f"最终按机械检查算{word[s['final_kept']]}。{s.get('note', '')}"
                for s in checked
                if s["judge_kept"] != s["my_kept"] or s["final_kept"] != s["my_kept"]
            ] or ["（没有。）"]
    else:
        lines.append("这次没有人工抽查。")
    lines += ["", "## 调用次数和花费", ""]
    lines.append("| 运行 | 调用次数（含重试） | 输入 token | 输出 token | 估算花费（美元） |")
    lines.append("| --- | --- | --- | --- | --- |")
    for label, meta in (("修改前", bm), ("修改后", am)):
        calls = meta.get("calls") or {}
        tin = sum(m["input_tokens"] for m in calls.get("models", {}).values())
        tout = sum(m["output_tokens"] for m in calls.get("models", {}).values())
        lines.append(f"| {label} | {calls.get('calls', 0)} | {tin} | {tout} | {calls.get('usd', 0):.2f} |")
    ledger = am.get("ledger")
    if ledger:
        tin = sum(m["input_tokens"] for m in ledger.get("models", {}).values())
        tout = sum(m["output_tokens"] for m in ledger.get("models", {}).values())
        lines.append(f"| 全部运行合计（含试跑和改提示词时的试验） | {ledger.get('calls', 0)} | {tin} | {tout} | {ledger.get('usd', 0):.2f} |")
    lines += ["", "每次运行的调用记在它自己的工作目录下：配对判定的调用记在「修改后」，它同时写下修改前和修改后的判定。"]
    lines += ["", "单价按 2026-11 的公开价估算（美元 / 百万 token）：" + "；".join(f"`{m}` 输入 {p[0]}、输出 {p[1]}" for m, p in PRICES.items()) + "。", ""]
    lines += ["## 判定提示词", "", "系统提示词：", "", "```text", JUDGE_SYSTEM, "```", ""]
    lines += [
        "每次判定的用户消息由 `judge_message` 组装：预设名称和用途（`samples.json` 的 `units`）、原文、要点清单，"
        "和这条样例修改前后的全部结果；相同的结果只出现一次，顺序按样例打乱，依次标为 A、B、C……，不注明来自哪一版。",
        "",
    ]
    lines += [
        "带 `any`、`all`、`none` 的要点由脚本机械检查，并且判定模型也要认为保留才算保留：机械检查找出缺了的数字、名称和改口前的说法，"
        "判定模型找出字还在、但意思变了或者放错了地方的情况（例如负责人对调、例外变成条件）。带 `verbatim` 的要点只看机械检查（逐字比较字母和数字，"
        "大小写改动算违背用途，不算遗漏）。其余要点由判定模型判断。",
        "",
    ]
    return "\n".join(lines)


def avg_prompt_tokens(records: dict, unit: str) -> str:
    values = [run["prompt_tokens"] for r in records.values() if r["unit"] == unit for run in r["runs"] if run.get("prompt_tokens")]
    return str(round(sum(values) / len(values))) if values else "?"


def pct(n: int, d: int) -> str:
    return f"{100 * n / d:.1f}%" if d else "—"


def cmd_report(args: argparse.Namespace) -> None:
    data = load_samples()
    before, after = load_results(Path(args.before)), load_results(Path(args.after))
    spot = json.loads(Path(args.spot_check).read_text()) if args.spot_check else None
    remarks = json.loads(Path(args.remarks).read_text()) if getattr(args, "remarks", None) else None
    Path(args.report).write_text(render_report(before, after, data, spot, remarks), encoding="utf-8")
    for key in MODELS:
        if key in before["models"] and key in after["models"]:
            result = compare(before, after, unit_order(data), key)
            print(f"[report] {key}: omissions {result['before'].omissions} → {result['after'].omissions}, goal {'met' if result['goal'] else 'NOT met'}", file=sys.stderr)


def cmd_record(args: argparse.Namespace) -> None:
    """docs/refine-eval/prompt-digests.json: the digests of the prompts the results measured."""
    meta = json.loads((Path(args.results) / "meta.json").read_text())
    commit = meta.get("commit", "")
    if commit.endswith("+dirty"):
        print("[record] note: the results came from a working tree with uncommitted prompt changes", file=sys.stderr)
    digests = json.loads(eval_tool("digests"))
    stale = changed_units(meta.get("digests") or {}, digests)
    if stale:
        raise EvalError(f"{args.results} measured other prompts or samples than the code has ({', '.join(stale)}); run generate and judge again")
    record = {
        "evaluated_at": meta.get("generated_at"),
        "report": str(Path(args.report).resolve().relative_to(REPO)),
        "results": str(Path(args.results).resolve().relative_to(REPO)),
        "models": sorted(meta.get("models", {}).values()),
        "judge": JUDGE_MODEL,
        "runs": meta.get("runs"),
        "command": "make refine-eval",
        "units": digests,
    }
    RECORD.parent.mkdir(parents=True, exist_ok=True)
    RECORD.write_text(json.dumps(record, ensure_ascii=False, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    print(f"[record] {RECORD.relative_to(REPO)}: {len(digests)} units", file=sys.stderr)


def cmd_all(args: argparse.Namespace) -> None:
    """Measure the current prompts and compare them with the last committed results: those become
    results/before and are judged again beside the new answers, the new answers results/after."""
    out = Path(args.out)
    if out.exists():
        shutil.rmtree(out)
    previous, before = RESULTS / "after", RESULTS / "before"
    pair = None
    if previous.exists():
        if before.exists():
            shutil.rmtree(before)
        shutil.copytree(previous, before)
        pair = str(before)
    cmd_generate(args)
    cmd_judge(argparse.Namespace(**{**vars(args), "only": None, "force": False, "pair_with": pair}))
    cmd_save(argparse.Namespace(out=str(out), to=str(previous), note=None))
    if pair:
        cmd_report(argparse.Namespace(before=pair, after=str(previous), report=str(REPORT), spot_check=None, remarks=None))
    cmd_record(argparse.Namespace(results=str(previous), report=str(REPORT)))


def cmd_calls(args: argparse.Namespace) -> None:
    entries = []
    for d in args.dirs:
        path = Path(d)
        entries += read_jsonl(path / "calls.jsonl" if path.is_dir() else path)
    print(json.dumps(summarize_calls(entries), ensure_ascii=False, indent=1))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    def network(p: argparse.ArgumentParser) -> None:
        p.add_argument("--out", default=str(WORK))
        p.add_argument("--models", default=",".join(MODELS))
        p.add_argument("--only")
        p.add_argument("--workers", type=int, default=8)
        p.add_argument("--max-calls", type=int, default=MAX_CALLS)
        p.add_argument("--profile", default=PROFILE)
        p.add_argument("--region", default=REGION)

    g = sub.add_parser("generate")
    network(g)
    g.add_argument("--runs", type=int, default=RUNS)
    j = sub.add_parser("judge")
    network(j)
    j.add_argument("--pair-with", help="另一组提示词的工作目录：同一条样例的两组结果放在同一次判定里，两边的判定都写下")
    j.add_argument("--force", action="store_true")
    s = sub.add_parser("save")
    s.add_argument("--out", default=str(WORK))
    s.add_argument("--to", required=True)
    s.add_argument("--note", help="一句说明，写进结果和报告（例如这一版提示词来自哪个提交）")
    r = sub.add_parser("report")
    r.add_argument("--before", required=True)
    r.add_argument("--after", required=True)
    r.add_argument("--report", default=str(REPORT))
    r.add_argument("--spot-check")
    r.add_argument("--remarks", help="JSON：样例 id → 仍有遗漏的原因，写在报告里这条样例的下面")
    rec = sub.add_parser("record")
    rec.add_argument("--results", default=str(RESULTS / "after"))
    rec.add_argument("--report", default=str(REPORT))
    a = sub.add_parser("all")
    network(a)
    a.add_argument("--runs", type=int, default=RUNS)
    rc = sub.add_parser("recheck")
    rc.add_argument("--out", default=str(WORK))
    rc.add_argument("--models", default=",".join(MODELS))
    c = sub.add_parser("calls")
    c.add_argument("dirs", nargs="+")
    args = parser.parse_args(argv)
    commands = {
        "generate": cmd_generate,
        "judge": cmd_judge,
        "save": cmd_save,
        "report": cmd_report,
        "record": cmd_record,
        "all": cmd_all,
        "recheck": cmd_recheck,
        "calls": cmd_calls,
    }
    try:
        commands[args.command](args)
    except EvalError as e:
        print(f"refine_eval: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
