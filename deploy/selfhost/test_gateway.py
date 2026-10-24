"""The self-hosted recognition gateway (deploy/selfhost/gateway/) in the nginx image compose.yaml runs.

Starts the gateway on the host's network in front of a stub of vLLM's API, with the template, the
token script and a tokens file mounted as compose.yaml mounts them, and checks what it lets through:
only requests with a listed token, only the two endpoints Voltip uses, uploads within the limit, a
limited number of requests per client address. Needs Docker and the host's network; skipped without
Docker except on CI, where a missing Docker fails.

    python3 -m unittest deploy/selfhost/test_gateway.py
"""

import hashlib
import http.server
import json
import os
import re
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
COMPOSE = (HERE / "compose.yaml").read_text(encoding="utf-8")
IMAGE = re.search(r"\$\{VOLTIP_NGINX_IMAGE:-([^}]+)\}", COMPOSE).group(1)

GOOD = "vasr_" + "A1b2C3d4" * 5
OTHER = "other-token.with~chars+/="
TOKENS = "\n".join(
    [
        "# the gateway's test tokens",  # line 1
        "short123",  # line 2: too short, skipped
        "has a space inside",  # line 3: not a token, skipped
        GOOD,  # line 4
        f"  {OTHER}  # with a comment",  # line 5
    ]
)
UPLOAD_LIMIT_MB = 1
TIMEOUT_S = 2
BURST = 3


def docker_ready() -> bool:
    if shutil.which("docker") is None:
        return False
    return subprocess.run(["docker", "info"], capture_output=True, timeout=60).returncode == 0


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Vllm(http.server.ThreadingHTTPServer):
    """A stub of vLLM's API that records what reaches it."""

    daemon_threads = True

    def __init__(self) -> None:
        super().__init__(("127.0.0.1", 0), Handler)
        self.seen: list[dict] = []
        self.lock = threading.Lock()

    def handle_error(self, request, client_address) -> None:
        # The slow answer's connection is gone when it is written: nothing to report.
        pass


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args) -> None:  # quiet
        pass

    def record(self, body: bytes) -> None:
        with self.server.lock:
            self.server.seen.append(
                {
                    "method": self.command,
                    "path": self.path,
                    "authorization": self.headers.get("Authorization"),
                    "sha256": hashlib.sha256(body).hexdigest(),
                }
            )

    def answer(self, status: int, payload: dict | None) -> None:
        data = b"" if payload is None else json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self) -> None:
        self.record(b"")
        if self.path == "/health":
            self.answer(200, None)
        elif self.path == "/v1/models":
            self.answer(200, {"object": "list", "data": [{"id": "Qwen/Qwen3-ASR-1.7B", "object": "model"}]})
        else:
            self.answer(404, {"detail": "not found"})

    def do_POST(self) -> None:
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        self.record(body)
        if b"name=\"model\"\r\n\r\nslow" in body:
            time.sleep(TIMEOUT_S + 2)
        self.answer(200, {"text": "你好", "sha256": hashlib.sha256(body).hexdigest()})


class Gateway:
    """One gateway container on the host's network."""

    def __init__(self, tokens: Path, asr_port: int, rate: int = 1, burst: int = BURST) -> None:
        self.port = free_port()
        self.name = f"voltip-gateway-test-{uuid.uuid4().hex[:12]}"
        env = {
            "VOLTIP_GATEWAY_PORT": self.port,
            "VOLTIP_ASR_PORT": asr_port,
            "VOLTIP_RATE": rate,
            "VOLTIP_BURST": burst,
            "VOLTIP_CONNECTIONS": 4,
            "VOLTIP_MAX_UPLOAD_MB": UPLOAD_LIMIT_MB,
            "VOLTIP_TIMEOUT": TIMEOUT_S,
            "NGINX_ENTRYPOINT_QUIET_LOGS": 1,
        }
        command = ["docker", "run", "-d", "--rm", "--network", "host", "--name", self.name]
        for key, value in env.items():
            command += ["-e", f"{key}={value}"]
        command += [
            "-v", f"{HERE / 'gateway/default.conf.template'}:/etc/nginx/templates/default.conf.template:ro",
            "-v", f"{HERE / 'gateway/40-voltip-tokens.sh'}:/docker-entrypoint.d/40-voltip-tokens.sh:ro",
            "-v", f"{tokens}:/etc/voltip/tokens:ro",
            IMAGE,
        ]
        subprocess.run(command, check=True, capture_output=True, timeout=120)

    def wait(self, status: int) -> None:
        deadline = time.monotonic() + 60
        while True:
            try:
                got = request(self, "GET", "/healthz", address="192.0.2.250")[0]
            except OSError:
                got = None
            if got == status:
                return
            if time.monotonic() > deadline:
                raise AssertionError(f"/healthz answered {got}, not {status}:\n{self.logs()}")
            time.sleep(0.2)

    def logs(self) -> str:
        done = subprocess.run(["docker", "logs", self.name], capture_output=True, text=True, timeout=60)
        return done.stdout + done.stderr

    def stop(self) -> None:
        subprocess.run(["docker", "rm", "-f", self.name], capture_output=True, timeout=60)


def request(gateway: Gateway, method: str, path: str, token: str | None = None, body: bytes | None = None,
            content_type: str | None = None, address: str = "198.51.100.1") -> tuple[int, dict, dict]:
    """(status, headers, JSON body) of one request, as from `address` (Caddy's X-Forwarded-For)."""
    req = urllib.request.Request(f"http://127.0.0.1:{gateway.port}{path}", data=body, method=method)
    req.add_header("X-Forwarded-For", address)
    if token is not None:
        req.add_header("Authorization", f"Bearer {token}")
    if content_type:
        req.add_header("Content-Type", content_type)
    try:
        with urllib.request.urlopen(req, timeout=30) as response:
            status, headers, data = response.status, dict(response.headers), response.read()
    except urllib.error.HTTPError as error:
        with error:
            status, headers, data = error.code, dict(error.headers), error.read()
    return status, headers, (json.loads(data) if data.strip() else {})


def multipart(fields: dict[str, bytes]) -> tuple[bytes, str]:
    boundary = uuid.uuid4().hex
    parts = []
    for name, value in fields.items():
        extra = '; filename="audio.wav"\r\nContent-Type: audio/wav' if name == "file" else ""
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"{extra}\r\n\r\n'.encode() + value + b"\r\n")
    return b"".join(parts) + f"--{boundary}--\r\n".encode(), f"multipart/form-data; boundary={boundary}"


@unittest.skipUnless(os.environ.get("CI") or docker_ready(), "needs Docker")
class GatewayTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        subprocess.run(["docker", "pull", "-q", IMAGE], check=True, capture_output=True, timeout=900)
        cls.vllm = Vllm()
        threading.Thread(target=cls.vllm.serve_forever, daemon=True).start()
        cls.dir = Path(tempfile.mkdtemp())
        cls.tokens = cls.dir / "tokens"
        cls.tokens.write_text(TOKENS + "\n", encoding="utf-8")
        cls.tokens.chmod(0o600)
        cls.gateway = Gateway(cls.tokens, cls.vllm.server_address[1])
        cls.addClassCleanup(cls.gateway.stop)
        cls.addClassCleanup(cls.vllm.server_close)
        cls.addClassCleanup(cls.vllm.shutdown)
        cls.addClassCleanup(shutil.rmtree, cls.dir, True)
        cls.gateway.wait(200)

    def seen(self) -> list[dict]:
        with self.vllm.lock:
            return list(self.vllm.seen)

    def test_a_request_without_a_listed_token_is_refused_before_vllm(self) -> None:
        before = len(self.seen())
        for token in (None, "vasr_not-one-of-the-listed-tokens", "short123", "has a space inside"):
            status, _, body = request(self.gateway, "GET", "/v1/models", token=token, address="198.51.100.10")
            self.assertEqual(status, 401, token)
            self.assertEqual(body["error"]["type"], "authentication_error")
        self.assertEqual(len(self.seen()), before, "vLLM never saw them")

    def test_a_listed_token_lists_the_models_and_vllm_never_sees_it(self) -> None:
        for token in (GOOD, OTHER):
            status, _, body = request(self.gateway, "GET", "/v1/models", token=token, address="198.51.100.11")
            self.assertEqual(status, 200, token)
            self.assertEqual(body["data"][0]["id"], "Qwen/Qwen3-ASR-1.7B")
        self.assertTrue(all(seen["authorization"] is None for seen in self.seen()), self.seen())

    def test_a_recording_reaches_vllm_as_it_was_sent(self) -> None:
        audio = os.urandom(200_000)
        body, content_type = multipart({"file": audio, "model": b"Qwen/Qwen3-ASR-1.7B", "prompt": b"kubectl, Voltip"})
        status, _, answer = request(self.gateway, "POST", "/v1/audio/transcriptions", GOOD, body, content_type, "198.51.100.12")
        self.assertEqual(status, 200)
        self.assertEqual(answer["text"], "你好")
        self.assertEqual(answer["sha256"], hashlib.sha256(body).hexdigest(), "the body is passed on byte for byte")
        last = self.seen()[-1]
        self.assertEqual((last["method"], last["path"], last["authorization"]), ("POST", "/v1/audio/transcriptions", None))

    def test_only_the_endpoints_voltip_uses_pass(self) -> None:
        before = len(self.seen())
        cases = [
            ("GET", "/v1/chat/completions", 404),
            ("POST", "/v1/completions", 404),
            ("GET", "/metrics", 404),
            ("GET", "/v1/audio/transcriptions", 405),
            ("POST", "/v1/models", 405),
        ]
        for i, (method, path, expected) in enumerate(cases):
            body = b"{}" if method == "POST" else None
            status, _, body = request(self.gateway, method, path, GOOD, body, "application/json", f"198.51.100.{40 + i}")
            self.assertEqual(status, expected, f"{method} {path}")
            self.assertIn("error", body)
        self.assertEqual(len(self.seen()), before, "vLLM never saw them")

    def test_an_upload_over_the_limit_is_refused(self) -> None:
        body, content_type = multipart({"file": b"\0" * (UPLOAD_LIMIT_MB * 1024 * 1024 + 1), "model": b"m"})
        status, _, answer = request(self.gateway, "POST", "/v1/audio/transcriptions", GOOD, body, content_type, "198.51.100.14")
        self.assertEqual(status, 413)
        self.assertIn(f"{UPLOAD_LIMIT_MB} MB", answer["error"]["message"])

    def test_each_client_address_is_limited_on_its_own(self) -> None:
        statuses = []
        for _ in range(BURST + 3):
            status, headers, body = request(self.gateway, "GET", "/v1/models", GOOD, address="198.51.100.15")
            statuses.append(status)
            if status == 429:
                self.assertEqual(headers.get("Retry-After"), "5")
                self.assertEqual(body["error"]["type"], "rate_limit")
        self.assertIn(429, statuses)
        self.assertEqual(statuses[0], 200)
        # Another address is not held back by the first one's requests.
        self.assertEqual(request(self.gateway, "GET", "/v1/models", GOOD, address="198.51.100.16")[0], 200)

    def test_a_slow_answer_ends_with_504(self) -> None:
        body, content_type = multipart({"file": b"RIFF", "model": b"slow"})
        status, _, answer = request(self.gateway, "POST", "/v1/audio/transcriptions", GOOD, body, content_type, "198.51.100.17")
        self.assertEqual(status, 504)
        self.assertEqual(answer["error"]["type"], "timeout")

    def test_the_log_names_the_client_and_the_tokens_line_never_the_token(self) -> None:
        request(self.gateway, "GET", "/v1/models", OTHER, address="203.0.113.9")
        deadline = time.monotonic() + 10
        while True:
            logs = self.gateway.logs()
            lines = [json.loads(line) for line in logs.splitlines() if line.startswith("{")]
            mine = [line for line in lines if line["address"] == "203.0.113.9"]
            if mine or time.monotonic() > deadline:
                break
            time.sleep(0.2)
        self.assertTrue(mine, logs)
        self.assertEqual((mine[-1]["token"], mine[-1]["path"], mine[-1]["status"]), ("t5", "/v1/models", 200))
        self.assertIn("2 access token(s) loaded", logs)
        self.assertIn("line 2 of the tokens file is shorter than 16 characters", logs)
        self.assertNotIn(GOOD, logs)
        self.assertNotIn(OTHER, logs)


@unittest.skipUnless(os.environ.get("CI") or docker_ready(), "needs Docker")
class GatewayWithoutVllmTest(unittest.TestCase):
    """While vLLM is down or still starting, the gateway says so."""

    @classmethod
    def setUpClass(cls) -> None:
        subprocess.run(["docker", "pull", "-q", IMAGE], check=True, capture_output=True, timeout=900)
        cls.dir = Path(tempfile.mkdtemp())
        tokens = cls.dir / "tokens"
        tokens.write_text(GOOD + "\n", encoding="utf-8")
        cls.gateway = Gateway(tokens, free_port())
        cls.addClassCleanup(cls.gateway.stop)
        cls.addClassCleanup(shutil.rmtree, cls.dir, True)
        cls.gateway.wait(503)

    def test_health_and_requests_answer_503_with_retry_after(self) -> None:
        for path, token in (("/healthz", None), ("/v1/models", GOOD)):
            status, headers, body = request(self.gateway, "GET", path, token, address="198.51.100.20")
            self.assertEqual(status, 503, path)
            self.assertEqual(headers.get("Retry-After"), "10")
            self.assertEqual(body["error"]["type"], "unavailable")


if __name__ == "__main__":
    unittest.main()
