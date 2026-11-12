"""Docker DNS/준비 상태를 조회해 Caddy 표준 /load API에 정상 static upstream만 반영한다."""

from concurrent.futures import ThreadPoolExecutor
import http.client
import ipaddress
import json
import os
from pathlib import Path
import signal
import socket
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROLES = ("gateway", "inference")
POLL_SECONDS = 0.25
HEARTBEAT_SECONDS = 2
PROBES = ThreadPoolExecutor(max_workers=8)


def probe(address):
    connection = http.client.HTTPConnection(address, 9090, timeout=0.5)
    try:
        connection.request("GET", "/status", headers={"Connection": "close"})
        response = connection.getresponse()
        status = json.loads(response.read(16384))
        if response.status != 200 or not isinstance(status, dict):
            raise ValueError("준비 응답 불일치")
        return {"ip": address, **status}
    except (OSError, ValueError, http.client.HTTPException) as error:
        return {"ip": address, "ready": False, "error_type": type(error).__name__}
    finally:
        connection.close()


def discover(role):
    try:
        # 응용 cache 없이 Docker embedded DNS의 모든 A를 조회한다.
        addresses = sorted({item[4][0] for item in
                            socket.getaddrinfo(role, 8080, socket.AF_INET, socket.SOCK_STREAM)})
    except socket.gaierror as error:
        # 조회 실패를 이전 목록 유지로 바꾸면 종료된 서버로 계속 라우팅할 수 있다.
        # 풀 전체를 비우고 503으로 전환한다. 추론 요청 재시도로 실패를 숨기지 않는다.
        return {"role": role, "docker_a": [], "eligible_a": [], "instances": [],
                "dns_error": type(error).__name__}
    instances = list(PROBES.map(probe, addresses))
    return {"role": role, "docker_a": addresses,
            "eligible_a": [item["ip"] for item in instances
                           if item.get("ready") is True and item.get("role") == role],
            "instances": instances}


def proxy_handler(addresses, timeout):
    # 주소는 DNS가 반환한 IPv4만 받는다. 설정 텍스트에 요청/임의 호스트를 삽입하지 않는다.
    upstreams = " ".join(f"{ipaddress.IPv4Address(address)}:8080" for address in addresses)
    return f"""reverse_proxy {upstreams} {{
        lb_policy round_robin
        lb_retries 0
        transport http {{
            dial_timeout 2s
            response_header_timeout {timeout}s
            keepalive off
        }}
    }}"""


class Controller:
    def __init__(self):
        self.lock = threading.Lock()
        self.stopping = threading.Event()
        self.bootstrap = Path("/stage2/Caddyfile").read_text()
        self.admin = os.environ["STAGE2_ADMIN_IP"]
        self.generation = 0
        self.applied = None
        self.expected_etag = None
        self.last_confirmed = 0.0
        self.error = "initializing"
        self.snapshots = {role: {"role": role, "docker_a": [], "eligible_a": [],
                                  "instances": [], "applied_a": [], "generation": 0}
                          for role in ROLES}

    def config(self, pools):
        config = self.bootstrap
        for role in ROLES:
            marker = f'respond "{role}_unavailable" 503'
            if config.count(marker) != 1:
                raise ValueError("Caddy bootstrap handler 경계 불일치")
            if pools[role]:
                config = config.replace(marker, proxy_handler(pools[role], 150 if role == "gateway" else 140))
        return config.encode()

    def admin_request(self, method, path, body=None):
        connection = http.client.HTTPConnection(self.admin, 2019, timeout=3)
        try:
            connection.request(method, path, body=body, headers={
                "Content-Type": "text/caddyfile", "Connection": "close",
            })
            response = connection.getresponse()
            response.read(262144)
            if response.status != 200:
                raise RuntimeError("Caddy 관리 API 오류")
            return response.getheader("Etag")
        finally:
            connection.close()

    def load(self, pools):
        # 같은 프로세스만 /load를 쓴다. 완료 후 실제 /config ETag로 재시작/외부 변경을 탐지한다.
        self.admin_request("POST", "/load", self.config(pools))
        etag = self.admin_request("GET", "/config/")
        if not etag:
            raise RuntimeError("Caddy 실제 설정 ETag 누락")
        with self.lock:
            self.expected_etag = etag
            self.generation += 1
            self.applied = pools
        print(json.dumps({"event": "upstreams_applied", "generation": self.generation,
                          "gateway_count": len(pools["gateway"]), "inference_count": len(pools["inference"])}), flush=True)

    def permitted(self):
        with self.lock:
            return (not self.stopping.is_set() and self.error is None
                    and time.monotonic() - self.last_confirmed < HEARTBEAT_SECONDS)

    def status(self, role):
        with self.lock:
            value = {**self.snapshots[role], "controller_error": self.error}
        return {**value, "permitted": self.permitted()}

    def run(self):
        while not self.stopping.is_set():
            try:
                snapshots = {role: discover(role) for role in ROLES}
                pools = {role: snapshots[role]["eligible_a"] for role in ROLES}
                actual_etag = self.admin_request("GET", "/config/")
                if pools != self.applied or actual_etag != self.expected_etag:
                    self.load(pools)
                with self.lock:
                    for role in ROLES:
                        self.snapshots[role] = {**snapshots[role], "applied_a": self.applied[role],
                                                "generation": self.generation}
                    recovered = self.error is not None
                    self.error = None
                    self.last_confirmed = time.monotonic()
                if recovered:
                    print('{"event":"controller_synchronized"}', flush=True)
            except Exception as error:
                # 재동기화는 제어면 reconciliation이다. 실제 사용자 HTTP 요청을 재전송하지 않는다.
                # API 장애/루프 정지/프로세스 kill 때 permit이 닫히므로 finally에 안전을 의존하지 않는다.
                with self.lock:
                    changed = self.error != type(error).__name__
                    self.error = type(error).__name__
                if changed:
                    print(json.dumps({"event": "controller_unavailable",
                                      "error_type": type(error).__name__}), flush=True)
            self.stopping.wait(POLL_SECONDS)


class StatusHandler(BaseHTTPRequestHandler):
    controller = None

    def log_message(self, format, *args):
        pass

    def do_GET(self):
        if self.path not in ("/gateway", "/inference"):
            self.send_error(404)
            return
        body = json.dumps(self.controller.status(self.path[1:])).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class PermitHandler(BaseHTTPRequestHandler):
    controller = None

    def log_message(self, format, *args):
        pass

    def do_GET(self):
        # 이 관리망 리스너는 상태 판정만 한다. 프록시·관리 API 전달·요청 본문 처리가 없다.
        forbidden = any(name.lower() in ("authorization", "cookie") or name.lower().startswith("x-forwarded-")
                        for name in self.headers)
        clean = (self.path == "/permit" and not forbidden
                 and self.headers.get("Content-Length", "0") == "0"
                 and self.headers.get("Transfer-Encoding") is None)
        if not clean:
            print('{"event":"unsafe_permit_request"}', flush=True)
        code = 200 if clean and self.controller.permitted() else 503
        self.send_response(code)
        self.send_header("Content-Length", "0")
        self.send_header("Connection", "close")
        self.end_headers()
        self.close_connection = True


if __name__ == "__main__":
    if "nameserver 127.0.0.11" not in Path("/etc/resolv.conf").read_text():
        raise RuntimeError("Docker embedded DNS 설정이 필요합니다")
    controller = Controller()
    signal.signal(signal.SIGTERM, lambda *_: controller.stopping.set())
    signal.signal(signal.SIGINT, lambda *_: controller.stopping.set())
    StatusHandler.controller = controller
    PermitHandler.controller = controller
    server = ThreadingHTTPServer(("127.0.0.1", 9091), StatusHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    permit = ThreadingHTTPServer((os.environ["STAGE2_CONTROLLER_IP"], 9092), PermitHandler)
    threading.Thread(target=permit.serve_forever, daemon=True).start()
    try:
        controller.run()
    finally:
        permit.shutdown()
        permit.server_close()
        server.shutdown()
        server.server_close()
        PROBES.shutdown(wait=True)
