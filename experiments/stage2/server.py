"""제품 Rust Gateway가 아닌 단계 2 전용 인증/추론 서버. 응답 의미는 원본을 유지한다."""

import hashlib
import hmac
import http.client
import importlib.metadata
import json
import os
from pathlib import Path
import re
import signal
import socket
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from uuid import uuid4

MODEL = Path("/models/decider/decider-2b-v12-Q4_K_M.gguf")
MODEL_SHA256 = "e6d3a7534a3abc3a723f8263dc694b86f533214a5bf761474113c66d44018340"
WITHDRAW_SECONDS = 8
MAX_BODY = 64 * 1024


class RequestError(Exception):
    """입력 검증 실패만 400으로 구분하고 모델 실행 실패를 숨기지 않는다."""


def event(name, **fields):
    # 예외 메시지에는 원문 입력이 들어갈 수 있어 종류만 기록한다.
    print(json.dumps({"event": name, **fields}, ensure_ascii=False), flush=True)


class HTTPServer(ThreadingHTTPServer):
    daemon_threads = False
    block_on_close = True

    def handle_error(self, request, client_address):
        event("http_error", error_type=sys.exc_info()[0].__name__)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def setup(self):
        super().setup()
        self.connection.settimeout(150)

    def log_message(self, format, *args):
        # 기본 http.server 로그는 요청 URI를 기록하므로 사용하지 않는다.
        pass

    def reply(self, status, body, headers=None):
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(body)
        self.wfile.flush()
        self.close_connection = True

    def fail(self, status, code):
        self.reply(status, json.dumps({"error": code}).encode())

    def send_error(self, code, message=None, explain=None):
        # BaseHTTPRequestHandler의 오류 본문도 요청 내용을 반사하지 않게 한다.
        self.fail(code, "http_error")


class Runtime:
    def __init__(self, role):
        self.role = role
        self.instance = f"{socket.gethostname()}-{uuid4().hex[:8]}"
        self.lock = threading.Lock()
        self.phase = "loading"
        self.active = set()
        self.completed = 0
        self.failed = 0
        self.terminating = threading.Event()
        self.admission = threading.Event()
        self.completion = threading.Event()
        self.completion.set()
        if os.environ.get("STAGE2_START_HELD", "0") == "0":
            self.admission.set()
        self.model_lock = threading.Lock()
        self.model = None
        self.key = b""

    def status(self):
        with self.lock:
            return {"role": self.role, "instance": self.instance,
                    "phase": self.phase, "ready": self.phase == "ready",
                    "active": sorted(self.active), "completed": self.completed,
                    "failed": self.failed}

    def phase_to(self, phase):
        with self.lock:
            self.phase = phase
        event("phase", role=self.role, instance=self.instance, phase=phase)

    def load(self):
        if self.role == "gateway":
            self.key = os.environ["STAGE2_API_KEY"].encode("ascii")
            if len(self.key) < 32:
                raise ValueError("실험 인증 키 길이 부족")
            return
        for name, version in (("decider-ai", "1.10.0"), ("llama-cpp-python", "0.3.36")):
            if importlib.metadata.version(name) != version:
                raise ValueError("검증 런타임 버전 불일치")
        with MODEL.open("rb") as stream:
            if hashlib.file_digest(stream, "sha256").hexdigest() != MODEL_SHA256:
                raise ValueError("Q4 모델 해시 불일치")
        import torch
        from decider.infer import Decider

        torch.set_num_threads(4)
        # 가중치·온도·isolated_levels를 바꾸지 않는다. 라이브러리 monkeypatch도 없다.
        self.model = Decider(str(MODEL), device="cpu", gguf_options={
            "n_gpu_layers": 0, "n_threads": 4, "n_ctx": 2048,
            "n_seq_max": 1, "verbose": True,
        })

    def accept(self, request_id):
        with self.lock:
            if self.phase not in ("ready", "withdrawing"):
                return False
            if request_id in self.active:
                return False
            self.active.add(request_id)
            return True

    def finish(self, request_id, success):
        with self.lock:
            self.active.remove(request_id)
            if success:
                self.completed += 1
            else:
                self.failed += 1

    def infer(self, raw):
        try:
            request = json.loads(raw)
        except (ValueError, UnicodeError) as error:
            raise RequestError("JSON 입력 오류") from error
        if not isinstance(request, dict) or set(request) != {"state", "questions"}:
            raise RequestError("실험 요청 필드 불일치")
        if not isinstance(request["state"], (str, dict)):
            raise RequestError("state 타입 불일치")
        questions = request["questions"]
        if not isinstance(questions, dict) or not 1 <= len(questions) <= 4:
            raise RequestError("질문 개수 불일치")
        for question in questions.values():
            if not isinstance(question, dict) or question.get("type") not in ("choice", "noul", "score"):
                raise RequestError("질문 타입 불일치")
        # 원본 GGUF 컨텍스트는 공유 동시 호출하지 않는다. 접수된 대기도 drain 대상이다.
        with self.model_lock:
            result = self.model.system_one(request["state"], questions, max_state_tokens=2048)
        body = json.dumps(result, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode()
        return 200, body, {"X-Stage2-Inference": self.instance,
                           "X-Stage2-Body-SHA256": hashlib.sha256(body).hexdigest()}

    def forward(self, raw, request_id):
        # 클라이언트 헤더·Host·Authorization은 내부로 전달하지 않는다. 목적지는 고정이다.
        connection = http.client.HTTPConnection(os.environ["STAGE2_PRIVATE_IP"], 18081, timeout=145)
        try:
            connection.request("POST", "/v1/systemone", body=raw, headers={
                "Content-Type": "application/json", "X-Stage2-Request-ID": request_id,
                "Connection": "close",
            })
            response = connection.getresponse()
            body = response.read(MAX_BODY * 4 + 1)
            if len(body) > MAX_BODY * 4:
                raise ValueError("추론 응답 한도 초과")
            headers = {name: response.getheader(name) for name in
                       ("X-Stage2-Inference", "X-Stage2-Body-SHA256") if response.getheader(name)}
            headers["X-Stage2-Gateway"] = self.instance
            return response.status, body, headers
        finally:
            connection.close()


def handlers(runtime):
    class DataHandler(Handler):
        def do_GET(self):
            if runtime.role == "gateway" and not self.authorized():
                self.fail(401, "unauthorized")
            else:
                self.fail(404, "not_found")

        def authorized(self):
            values = self.headers.get_all("Authorization", [])
            return len(values) == 1 and hmac.compare_digest(
                values[0].encode("utf-8"), b"Bearer " + runtime.key)

        def do_POST(self):
            # 인증 실패는 body 읽기·모델 실행보다 먼저 반환한다.
            if runtime.role == "gateway" and not self.authorized():
                self.fail(401, "unauthorized")
                return
            if self.path != "/v1/systemone":
                self.fail(404, "not_found")
                return
            lengths = self.headers.get_all("Content-Length", [])
            if self.headers.get("Transfer-Encoding") or len(lengths) != 1:
                self.fail(400, "invalid_framing")
                return
            try:
                length = int(lengths[0])
            except ValueError:
                self.fail(400, "invalid_length")
                return
            if not 0 < length <= MAX_BODY:
                self.fail(413, "body_limit")
                return
            request_id = self.headers.get("X-Stage2-Request-ID", uuid4().hex)
            if not re.fullmatch("[0-9a-f]{32}", request_id):
                self.fail(400, "invalid_request_id")
                return
            if not runtime.accept(request_id):
                self.fail(503, "not_accepting")
                return
            success = False
            try:
                raw = self.rfile.read(length)
                if len(raw) != length:
                    self.fail(400, "incomplete_body")
                    return
                if runtime.role == "gateway":
                    status, body, headers = runtime.forward(raw, request_id)
                else:
                    status, body, headers = runtime.infer(raw)
                # 실험 제어면의 완료 장벽만 응답을 대기시킨다. 실제 추론은 이미 수행했다.
                if not runtime.completion.wait(120):
                    raise TimeoutError("완료 장벽 시간 초과")
                headers["X-Stage2-Request-ID"] = request_id
                self.reply(status, body, headers)
                success = status == 200
            except RequestError:
                event("request_error", role=runtime.role, error_type="invalid_request")
                self.fail(400, "invalid_request")
            except Exception as error:
                event("request_error", role=runtime.role, error_type=type(error).__name__)
                self.fail(502 if runtime.role == "gateway" else 500, "request_failed")
            finally:
                runtime.finish(request_id, success)

    class ControlHandler(Handler):
        def do_GET(self):
            if self.path not in ("/status", "/ready"):
                self.fail(404, "not_found")
                return
            status = runtime.status()
            self.reply(200 if self.path == "/status" or status["ready"] else 503,
                       json.dumps(status).encode())

        def do_POST(self):
            # Docker exec를 통한 loopback 관리만 허용한다. 프록시 경로에는 노출하지 않는다.
            if self.client_address[0] != "127.0.0.1":
                self.fail(403, "loopback_only")
                return
            actions = {"/release-start": runtime.admission.set,
                       "/hold-completion": runtime.completion.clear,
                       "/release-completion": runtime.completion.set,
                       "/drain": runtime.terminating.set}
            if self.path not in actions:
                self.fail(404, "not_found")
                return
            actions[self.path]()
            self.reply(200, b"{}")

    return DataHandler, ControlHandler


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in ("gateway", "inference"):
        raise ValueError("gateway 또는 inference가 필요합니다")
    runtime = Runtime(sys.argv[1])
    signal.signal(signal.SIGTERM, lambda *_: runtime.terminating.set())
    signal.signal(signal.SIGINT, lambda *_: runtime.terminating.set())
    data_handler, control_handler = handlers(runtime)
    control = HTTPServer(("0.0.0.0", 9090), control_handler)
    threading.Thread(target=control.serve_forever, daemon=True).start()
    data = None
    try:
        runtime.load()
        runtime.phase_to("held")
        while not runtime.admission.wait(0.1):
            if runtime.terminating.is_set():
                return
        if runtime.terminating.is_set():
            return
        # 모델 로딩과 검사가 끝나기 전에는 추론 리스너를 만들지 않는다.
        data = HTTPServer(("0.0.0.0", 8080), data_handler)
        threading.Thread(target=data.serve_forever, daemon=True).start()
        runtime.phase_to("ready")
        runtime.terminating.wait()
        runtime.phase_to("withdrawing")
        # controller의 DNS/probe/API timeout과 poll을 포함한 전파 여유다.
        # 실제 제외 완료는 smoke에서 applied_a로 관측한다. 관리망 장애를 무중단으로 간주하지 않는다.
        time.sleep(WITHDRAW_SECONDS)
        runtime.phase_to("draining")
        data.shutdown()
        data.server_close()
        data = None
        runtime.phase_to("stopped")
    finally:
        if data is not None:
            data.shutdown()
            data.server_close()
        control.shutdown()
        control.server_close()


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        event("fatal", error_type=type(error).__name__)
        sys.exit(1)
