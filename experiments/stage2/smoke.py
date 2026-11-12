"""호스트의 실제 TLS·인증·Decider 검사. 승인된 picksy-stage2 자원에만 scale/recreate를 적용한다."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import http.client
import ipaddress
import json
import math
import os
from pathlib import Path
import socket
import ssl
import subprocess
import sys
import time
from uuid import uuid4

COMPOSE = ["docker", "compose", "-p", "picksy-stage2", "-f", str(Path(__file__).with_name("compose.yaml"))]
PROJECT = "picksy-stage2"
RUNTIME_IMAGE = "sha256:c6632d172bb586d377bfebd7625348f1daf981818f325338c125f92aeab9f41f"
ROLES = ("gateway", "inference")
FIXTURES = {
    "choice": {"state": "오류 수정 없이 다크 모드 선택 기능을 추가했다.", "questions": {
        "result": {"type": "choice", "instructions": "변경의 주목적은?", "criteria": {
            "fix": "기존 오류 수정", "feature": "새 기능 추가", "docs": "문서만 수정"}}}},
    "noul": {"state": "요구: 빈 입력은 400. 결과: 빈 입력에 400 반환.", "questions": {
        "result": {"type": "noul", "instructions": "요구사항을 충족했는가?"}}},
    "score": {"state": "적에게 맞아 체력이 계속 감소한다.", "questions": {
        "result": {"type": "score", "instructions": "즉시 대응의 필요성은?", "criteria": [
            "안전: 현재 공격도 위험도 없음", "경계: 적이 접근하지만 아직 공격하지 않음",
            "긴급: 현재 피해를 받고 있음"]}}},
}


class CheckError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise CheckError(message)


def report(check, **metadata):
    # 합성 본문조차 출력하지 않는다. 인프라 식별자·관측 결과만 보고한다.
    print(json.dumps({"check": check, **metadata}, ensure_ascii=False), flush=True)


def command(arguments, *, env=None, timeout=180):
    result = subprocess.run(arguments, env=env, capture_output=True, text=True, timeout=timeout)
    if result.returncode:
        # Compose 환경/오류에 비밀값이 포함될 가능성이 있어 원문 stderr를 반사하지 않는다.
        raise CheckError(f"명령 실패: {arguments[0]} (exit={result.returncode})")
    return result.stdout.strip()


def containers(role, *, all_containers=False):
    args = ["docker", "ps", "-q"]
    if all_containers:
        args.append("-a")
    args += ["--filter", f"label=com.docker.compose.project={PROJECT}",
             "--filter", f"label=com.docker.compose.service={role}",
             "--filter", "label=com.docker.compose.oneoff=False"]
    return command(args).split()


def only(role):
    ids = containers(role)
    require(len(ids) == 1, f"{role} 실행 컨테이너가 정확히 하나여야 함")
    return ids[0]


def inspect(container, field):
    # 전체 inspect는 Config.Env의 인증 키를 노출하므로 필요한 필드만 가져온다.
    return json.loads(command(["docker", "inspect", "--format", "{{json " + field + "}}", container]))


def control(container, action="status", role=None):
    args = ["docker", "exec", container, "python", "-B", "/stage2/control.py", action]
    if role:
        args += ["--role", role]
    return json.loads(command(args, timeout=10))


def statuses(role):
    return {container: control(container) for container in containers(role)}


def wait_for(predicate, description, timeout=90):
    # 이 루프는 제어면의 상태 전이만 기다린다. 사용자 HTTP 추론 요청은 절대 재시도하지 않는다.
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.2)
    raise CheckError(f"상태 대기 시간 초과: {description}")


def wait_phase(container, phases):
    def probe():
        try:
            status = control(container)
        except CheckError:
            state = inspect(container, ".State")
            require(state["Running"], "준비 전에 컨테이너 종료")
            return None
        return status if status["phase"] in phases else None
    return wait_for(probe, "/".join(phases))


def address(container, role):
    network = "gateways" if role == "gateway" else "inference"
    return inspect(container, ".NetworkSettings.Networks")[f"{PROJECT}_{network}"]["IPAddress"]


def pool(role, allow_unready=False):
    # Caddy에 공급되는 목록과 원시 Docker A를 따로 관측한다.
    value = control(only("controller"), "snapshot", role)
    if not allow_unready:
        require(value["controller_error"] is None and value["permitted"], "controller 준비/heartbeat 실패")
    return value


def wait_pool(role, ips):
    def converged():
        try:
            value = pool(role, allow_unready=True)
        except CheckError:
            require(inspect(only("controller"), ".State")["Running"], "controller 준비 전 종료")
            return None
        if value["controller_error"] is not None or not value["permitted"]:
            return None
        return value if set(value["applied_a"]) == set(ips) and set(value["docker_a"]) == set(ips) else None
    return wait_for(converged, f"{role} Docker DNS/관리 API 반영")


def replica_two(role):
    for container in containers(role):
        labels = inspect(container, ".Config.Labels")
        if labels.get("com.docker.compose.container-number") == "2":
            return container
    raise CheckError(f"{role} Compose 두 번째 복제본 없음")


def internal_tcp(container, host, port):
    result = command(["docker", "exec", container, "python", "-B", "/stage2/control.py", "tcp",
                      "--host", host, "--port", str(port)])
    return json.loads(result)["connected"]



def check_scope():
    for role in ("caddy", "controller", *ROLES):
        ids = containers(role, all_containers=True)
        require(bool(ids), f"{role} Compose 서비스가 먼저 기동되어야 함")
        for container in ids:
            require(inspect(container, ".Config.Labels").get("io.picksy.stage2") == "true",
                    "승인된 단계 2 자원이 아님: 변경하지 않음")


def ingress_probe(public_ip):
    # 실제 공개 bridge에만 붙는 일회성 클라이언트다. 모델·인증 키·Docker 소켓을 주지 않는다.
    source = """import json, socket, sys
result = {}
for port in (443, 18081, 2019):
    try:
        connection = socket.create_connection((sys.argv[1], port), timeout=1)
    except OSError:
        result[str(port)] = False
    else:
        connection.close()
        result[str(port)] = True
print(json.dumps(result))
"""
    name = f"{PROJECT}-boundary-{uuid4().hex[:8]}"
    raw = command(["docker", "run", "--name", name, "--pull", "never",
                   "--network", f"{PROJECT}_ingress", "--read-only", "--cap-drop", "ALL",
                   "--security-opt", "no-new-privileges:true", "--memory", "64m",
                   "--label", "io.picksy.stage2=true", "--entrypoint", "python",
                   RUNTIME_IMAGE, "-B", "-c", source, public_ip])
    require(json.loads(raw) == {"443": True, "18081": False, "2019": False},
            "공개 네트워크에서 Caddy 리스너 접근 경계 불일치")
    report("ingress_external_boundary", probe_container=name, public_tls_reachable=True,
           private_listener_blocked=True, admin_blocked=True)

class Client:
    def __init__(self):
        self.host = "localhost"
        self.port = int(os.environ.get("STAGE2_HTTPS_PORT", "18080"))
        self.key = os.environ["STAGE2_API_KEY"]
        def certificate():
            try:
                return command(["docker", "exec", only("caddy"), "cat",
                                "/data/caddy/pki/authorities/local/root.crt"])
            except CheckError:
                require(inspect(only("caddy"), ".State")["Running"], "CA 생성 전 Caddy 종료")
                return None
        ca = wait_for(certificate, "실험 TLS CA 생성")
        self.context = ssl.create_default_context(cadata=ca)
        self.ca_fingerprint = hashlib.sha256(ca.encode()).hexdigest()

    def request(self, kind="noul", *, authorization="valid", payload=None, context=None):
        request_id = uuid4().hex
        body = json.dumps(FIXTURES[kind], ensure_ascii=False).encode() if payload is None else payload
        headers = {"Content-Type": "application/json", "Connection": "close",
                   "X-Stage2-Request-ID": request_id,
                   "X-Stage2-Gateway": "untrusted", "X-Stage2-Inference": "untrusted"}
        if authorization != "missing":
            headers["Authorization"] = "Bearer " + (self.key if authorization == "valid" else "invalid")
        connection = http.client.HTTPSConnection(self.host, self.port, context=context or self.context, timeout=160)
        try:
            connection.request("POST", "/v1/systemone", body=body, headers=headers)
            response = connection.getresponse()
            raw = response.read()
            metadata = {"gateway": response.getheader("X-Stage2-Gateway"),
                        "inference": response.getheader("X-Stage2-Inference"),
                        "request_id": response.getheader("X-Stage2-Request-ID")}
            if response.status == 200:
                require(metadata["request_id"] == request_id, "요청 ID 전달 불일치")
                require(all(metadata[role] and metadata[role] != "untrusted" for role in ROLES),
                        "서버 instance 헤더 누락/위조 헤더 반사")
                require(hashlib.sha256(raw).hexdigest() == response.getheader("X-Stage2-Body-SHA256"),
                        "Gateway에서 원본 응답 byte 변경")
                validate(json.loads(raw), kind)
            return response.status, metadata
        finally:
            connection.close()

    def infer(self, kind="noul"):
        status, metadata = self.request(kind)
        require(status == 200, f"실제 추론 HTTP {status}; 재시도하지 않음")
        return metadata


def validate(body, kind):
    require(isinstance(body.get("model"), str) and bool(body["model"]), "원본 model 누락")
    require(set(body["answers"]) == {"result"}, "답변 키 불일치")
    answer = body["answers"]["result"]
    require(answer["type"] == kind, "답변 타입 불일치")
    if kind == "noul":
        value = answer["noul"]
        require(type(value) in (int, float) and math.isfinite(value) and 0 <= value <= 1, "noul 범위 오류")
        return
    probabilities = answer["probabilities"]
    keys = {"0", "1", "2"} if kind == "score" else {"fix", "feature", "docs"}
    require(set(probabilities) == keys, "확률 키 불일치")
    require(all(type(value) in (int, float) and math.isfinite(value) and 0 <= value <= 1
                for value in probabilities.values()), "확률 범위 오류")
    require(abs(sum(probabilities.values()) - 1) <= 0.000151, "반올림 범위 밖의 확률 합")
    if kind == "score":
        value = answer["score"]
        require(type(value) in (int, float) and math.isfinite(value) and 0 <= value <= 2, "score 범위 오류")
        require(abs(value - sum(int(key) * p for key, p in probabilities.items())) <= 0.0052,
                "원본 score 기대값 의미 변경")
    else:
        require(answer["choice"] in probabilities, "choice 키 누락")
        require(probabilities[answer["choice"]] >= max(probabilities.values()) - 0.0001,
                "choice 최대 확률 불일치")


def distribution(client, expected):
    observed = {role: set() for role in ROLES}
    # 무작위 대량 부하 대신 안정된 두 주소의 round_robin을 네 요청으로 검사한다.
    for _ in range(4):
        metadata = client.infer()
        for role in ROLES:
            observed[role].add(metadata[role])
    for role in ROLES:
        require(observed[role] == expected[role], f"{role} 실제 분산/생존자 불일치")
    report("distribution", **{role: sorted(values) for role, values in observed.items()})


def expected_instances():
    return {role: {status["instance"] for status in statuses(role).values()} for role in ROLES}


def blocked(host, port):
    try:
        connection = socket.create_connection((host, port), timeout=0.6)
    except OSError:
        return
    connection.close()
    raise CheckError(f"내부 TCP 접근 가능: {host}:{port}")


def network_boundary():
    caddy = only("caddy")
    bindings = inspect(caddy, ".HostConfig.PortBindings")
    require(set(bindings) == {"443/tcp"}, "Caddy 내부 포트 호스트 publish 발견")
    for port in (18081, 2019):
        blocked("127.0.0.1", port)
    networks = inspect(caddy, ".NetworkSettings.Networks")
    public_ip = networks[f"{PROJECT}_ingress"]["IPAddress"]
    admin_ip = networks[f"{PROJECT}_management"]["IPAddress"]
    ingress_probe(public_ip)
    private_ip = networks[f"{PROJECT}_gateways"]["IPAddress"]
    controller_networks = inspect(only("controller"), ".NetworkSettings.Networks")
    for endpoint in controller_networks.values():
        blocked(endpoint["IPAddress"], 9092)
    for endpoint in networks.values():
        for port in (18081, 2019):
            blocked(endpoint["IPAddress"], port)
    for role in ROLES:
        for container in containers(role):
            require(not inspect(container, ".HostConfig.PortBindings"), f"{role} 호스트 publish 발견")
            require(f"{PROJECT}_management" not in inspect(container, ".NetworkSettings.Networks"),
                    f"{role} 관리망 접속 발견")
            for port in (8080, 9090):
                blocked(address(container, role), port)
            for host in (public_ip, private_ip, admin_ip):
                require(not internal_tcp(container, host, 2019), f"{role} Caddy 관리자 API 접근 가능")
            for endpoint in controller_networks.values():
                require(not internal_tcp(container, endpoint["IPAddress"], 9092),
                        f"{role} controller 관리망 permit 접근 가능")
            require(not internal_tcp(container, public_ip, 18081), "공개 Caddy IP로 내부 리스너 접근 가능")
            require(internal_tcp(container, private_ip, 18081) == (role == "gateway"),
                    "Gateway/추론망 내부 리스너 접근 경계 불일치")
    # controller도 공개망에는 없다. 소켓이 아니라 전용 관리망으로만 Caddy에 접속한다.
    controller = only("controller")
    require(f"{PROJECT}_ingress" not in inspect(controller, ".NetworkSettings.Networks"), "controller 공개망 연결")
    require(internal_tcp(controller, admin_ip, 2019), "controller 관리자 API 연결 불가")
    require(not internal_tcp(controller, public_ip, 2019), "공개 Caddy IP 관리자 API 노출")
    require(not internal_tcp(controller, public_ip, 18081), "공개 Caddy IP 내부 리스너 노출")
    report("host_external_boundary", private_ports_blocked=True, management_network_isolated=True,
           scope="현재 Docker Desktop 호스트/컨테이너; 다른 OS/원격 클라이언트는 별도 검증")


def baseline(client):
    for role in ROLES:
        wait_phase(only(role), {"ready"})
        wait_pool(role, [address(only(role), role)])
    version = command(["docker", "exec", only("caddy"), "caddy", "version"])
    require(version.startswith("v2.10.2"), "고정 Caddy 버전 불일치")
    modules = command(["docker", "exec", only("caddy"), "caddy", "list-modules"])
    require("http.reverse_proxy.upstreams.a" in modules.splitlines(), "표준 dynamic A 모듈 누락")
    try:
        client.request(context=ssl.create_default_context())
    except ssl.SSLCertVerificationError:
        pass
    else:
        raise CheckError("실험 CA 없이 TLS 검증 실패가 발생하지 않음")
    before = {key: value["completed"] for key, value in statuses("inference").items()}
    for auth in ("missing", "wrong"):
        status, _ = client.request(authorization=auth)
        require(status == 401, f"인증 실패 HTTP {status}")
    after = {key: value["completed"] for key, value in statuses("inference").items()}
    require(before == after, "인증 실패 요청이 추론에 도달함")
    require(client.request(payload=b"{")[0] == 400, "JSON 오류 상태 불일치")
    for kind in FIXTURES:
        metadata = client.infer(kind)
        report("real_inference", kind=kind, **metadata)
    network_boundary()
    report("baseline", tls_ca_verified=True, unauthorized=401, caddy_version=version.split()[0])


def scale_up(client, role):
    survivor = only(role)
    other = next(item for item in ROLES if item != role)
    other_ids = set(containers(other))
    survivor_status = wait_phase(survivor, {"ready"})
    # 기존 복제본은 재생성하지 않고 새 복제본만 입장 장벽으로 준비 전 상태를 관측한다.
    environment = {**os.environ, "STAGE2_START_HELD": "1"}
    command(COMPOSE + ["up", "-d", "--no-deps", "--no-recreate", "--scale", f"{role}=2", role], env=environment)
    ids = set(containers(role))
    require(len(ids) == 2 and survivor in ids, "독립 확장 컨테이너 개수 불일치")
    added = (ids - {survivor}).pop()
    wait_phase(added, {"held"})
    discovered = wait_for(lambda: (value if address(added, role) in value["docker_a"] else None)
                          if (value := pool(role)) else None, "미준비 복제본 Docker A 탐색")
    require(address(added, role) not in discovered["applied_a"], "미준비 복제본이 Caddy upstream에 포함됨")
    require(not internal_tcp(only("controller"), address(added, role), 8080),
            "준비 장벽 해제 전에 데이터 리스너가 열림")
    for _ in range(2):
        require(client.infer()[role] == survivor_status["instance"], "준비 전 복제본으로 라우팅됨")
    control(added, "release-start")
    wait_phase(added, {"ready"})
    wait_pool(role, [address(container, role) for container in ids])
    require(set(containers(other)) == other_ids, "반대 풀 복제본도 변경됨")
    distribution(client, expected_instances())
    report("compose_scale_up", role=role, replicas=2, added=added, added_ip=address(added, role),
           survivor=survivor)


def drain(client, role, target):
    current = statuses(role)
    require(len(current) == 2 and target == replica_two(role), "Compose 축소 대상은 두 번째 복제본이어야 함")
    target_instance = current[target]["instance"]
    target_ip = address(target, role)
    require(all(status["ready"] for status in current.values()), "drain 전 두 복제본 준비 필요")
    generation = pool(role)["generation"]
    other = next(item for item in ROLES if item != role)
    other_ids = set(containers(other))
    expected = expected_instances()
    expected[role].remove(target_instance)
    control(target, "hold-completion")
    executor = ThreadPoolExecutor(max_workers=4)
    requests = [executor.submit(client.infer) for _ in range(2)]
    try:
        active = wait_for(lambda: control(target)["active"], "실제 요청 접수")
        # wait를 먼저 연결해 Compose가 종료 컨테이너를 삭제해도 실제 종료 코드를 잃지 않는다.
        exited = executor.submit(command, ["docker", "wait", target], timeout=200)
        shrink = executor.submit(command, COMPOSE + [
            "up", "-d", "--no-deps", "--no-recreate", "--scale", f"{role}=1", role], timeout=200)
        wait_phase(target, {"withdrawing", "draining"})
        removed = wait_for(lambda: (value if target_ip not in value["applied_a"] else None)
                           if (value := pool(role)) else None, "관리 API 신규 라우팅 제외")
        require(removed["generation"] > generation, "진행 요청 중 Caddy reload 관측 누락")
        wait_phase(target, {"draining"})
        status = control(target)
        require(set(active) <= set(status["active"]), "진행 요청 완료 전에 종료됨")
        distribution(client, expected)
        control(target, "release-completion")
        results = [future.result(timeout=160) for future in requests]
        require(any(result[role] == target_instance for result in results), "종료 대상 진행 응답 누락")
        require(exited.result(timeout=200) == "0", "drain 비정상 종료/OOM")
        shrink.result(timeout=200)
        require(len(containers(role, all_containers=True)) == 1, "Compose 실제 축소/삭제 미완료")
        require(set(containers(other)) == other_ids, "독립 축소 중 반대 풀 변경")
        report("compose_scale_down_drain", role=role, removed_container=target, replicas=1,
               completed_inflight=True, caddy_reload_observed=True, exit_code=0)
    finally:
        # 실패 시 장벽만 해제한다. 실패 요청을 다시 전송하거나 컨테이너를 강제 삭제하지 않는다.
        if target in containers(role):
            control(target, "release-completion")
        executor.shutdown(wait=True)


def scaled(client, gateway, inference):
    expected_counts = {"gateway": gateway, "inference": inference}
    for role, count in expected_counts.items():
        require(len(containers(role, all_containers=True)) == count, f"{role} Compose 총 복제본 수 불일치")
        require(len(containers(role)) == count, f"{role} 실행 복제본 수 불일치")
        for container in containers(role):
            wait_phase(container, {"ready"})
        wait_pool(role, [address(container, role) for container in containers(role)])
    distribution(client, expected_instances())
    report("compose_scale_result", **expected_counts)


def rediscovery(client, role, old_ip, old_instance):
    current = statuses(role)
    require(len(current) == 2, "교체 검사는 준비된 복제본 두 개 필요")
    require(all(item["ready"] for item in current.values()), "교체 후 준비 확인 필요")
    ips = {address(container, role) for container in current}
    require(old_ip not in ips, "이전 IP가 재사용됨: IP 교체 검증으로 인정하지 않음")
    require(old_instance not in {item["instance"] for item in current.values()}, "이전 프로세스가 남아 있음")
    discovered = wait_pool(role, ips)
    require(set(discovered["eligible_a"]) == ips, "교체 IP 준비 목록 불일치")
    distribution(client, expected_instances())
    report("replacement_ip_rediscovery", role=role, retired_ip=old_ip,
           retired_instance=old_instance, current_ips=sorted(ips))


def replace_pool(client, role):
    old = statuses(role)
    require(len(old) == 2, "교체 검사는 복제본 두 개에서 시작해야 함")
    target = replica_two(role)
    old_ip, old_instance = address(target, role), old[target]["instance"]
    other = next(item for item in ROLES if item != role)
    other_ids = set(containers(other))
    # Compose는 rolling updater가 아니다. 두 복제본을 먼저 정상 stop한 계획 중단과
    # 진행 요청을 보존하는 개별 scale-down 시험을 구분한다. 동시에 세 모델을 띄우지 않는다.
    command(COMPOSE + ["stop", "--timeout", "180", role], timeout=200)
    empty = wait_pool(role, [])
    require(bool(empty.get("dns_error")), "빈 서비스의 Docker DNS 조회 실패 관측 누락")
    require(client.request()[0] == 503, "빈 풀 fail-closed 503 불일치")
    report("empty_pool_closed", role=role, http_status=503, dns_error=empty.get("dns_error"))
    command(COMPOSE + ["up", "-d", "--no-deps", "--force-recreate", "--scale", f"{role}=2", role],
            env={**os.environ, "STAGE2_START_HELD": "0"})
    current = containers(role)
    require(len(current) == 2 and not set(current) & set(old), "Compose force-recreate 미완료")
    for container in current:
        wait_phase(container, {"ready"})
    wait_pool(role, [address(container, role) for container in current])
    report("compose_force_recreate", role=role, old_containers=sorted(old), new_containers=sorted(current),
           planned_pool_outage=True)
    # Docker IP 재사용은 재생성 실패가 아니다. 주소 변화 검증만 별도 endpoint 재연결로 수행한다.
    for container in current:
        if address(container, role) != old_ip:
            continue
        network_name = f"{PROJECT}_{'gateways' if role == 'gateway' else 'inference'}"
        endpoint = inspect(container, ".NetworkSettings.Networks")[network_name]
        network = json.loads(command(["docker", "network", "inspect", network_name]))[0]
        subnet = ipaddress.ip_network(network["IPAM"]["Config"][0]["Subnet"])
        used = {item["IPv4Address"].split("/")[0] for item in network["Containers"].values()}
        new_ip = next((str(subnet.network_address + offset) for offset in range(200, 250)
                       if str(subnet.network_address + offset) not in used), None)
        require(new_ip is not None, "IP 교체용 미사용 주소 없음")
        command(["docker", "network", "disconnect", network_name, container])
        aliases = endpoint.get("Aliases") or [role]
        connect = ["docker", "network", "connect", "--ip", new_ip]
        for alias in aliases:
            connect += ["--alias", alias]
        command(connect + [network_name, container])
        require(address(container, role) == new_ip, "endpoint 실제 IP 변경 실패")
        report("endpoint_ip_changed", role=role, container=container, old_ip=old_ip, new_ip=new_ip,
               separate_from_container_recreation=True)
    require(set(containers(other)) == other_ids, "재생성 중 반대 풀 변경")
    rediscovery(client, role, old_ip, old_instance)


def control_faults(client):
    controller = only("controller")
    inference = only("inference")
    original = {role: set(containers(role)) for role in ROLES}
    control(inference, "hold-completion")
    executor = ThreadPoolExecutor(max_workers=1)
    active_request = executor.submit(client.infer)
    try:
        wait_for(lambda: control(inference)["active"], "장애 전 실제 추론 접수")
        command(["docker", "kill", "--signal", "KILL", controller])
        require(inspect(controller, ".State")["ExitCode"] == 137, "controller SIGKILL 관측 실패")
        status, _ = client.request()
        require(status in (502, 503), "controller kill 뒤 신규 요청이 차단되지 않음")
        control(inference, "release-completion")
        active_request.result(timeout=160)
        report("controller_kill_closed", new_request_status=status, admitted_request_completed=True)
    finally:
        control(inference, "release-completion")
        executor.shutdown(wait=True)
    command(["docker", "start", controller])
    for role in ROLES:
        wait_pool(role, [address(container, role) for container in containers(role)])
    client.infer()
    generation = pool("gateway")["generation"]
    before_ca = client.ca_fingerprint
    command(["docker", "restart", "--time", "180", only("caddy")], timeout=200)
    def synchronized():
        value = pool("gateway", allow_unready=True)
        return (value["controller_error"] is None and value["permitted"]
                and value["generation"] > generation)
    wait_for(synchronized, "Caddy 재시작 실제 설정 재동기화")
    for role in ROLES:
        wait_pool(role, [address(container, role) for container in containers(role)])
        require(set(containers(role)) == original[role], "제어면 복구 중 worker 교체")
    # tmpfs CA 변경 여부를 단정하지 않고 현재 공개 인증서를 다시 신뢰해 실제 TLS를 확인한다.
    client.__init__()
    client.infer()
    report("caddy_restart_resynchronized", worker_ids_unchanged=True,
           ca_changed=before_ca != client.ca_fingerprint,
           generation=pool("gateway")["generation"])


def full(client):
    baseline(client)
    control_faults(client)
    scale_up(client, "gateway")
    drain(client, "gateway", replica_two("gateway"))
    scaled(client, 1, 1)
    scale_up(client, "inference")
    drain(client, "inference", replica_two("inference"))
    scaled(client, 1, 1)
    scale_up(client, "gateway")
    scale_up(client, "inference")
    scaled(client, 2, 2)
    for role in ROLES:
        replace_pool(client, role)
    network_boundary()
    for role in ROLES:
        drain(client, role, replica_two(role))
    scaled(client, 1, 1)
    report("full_completed", gateway=1, inference=1, stopped=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    sub.add_parser("baseline")
    sub.add_parser("full")
    sub.add_parser("control-faults")
    recreate = sub.add_parser("replace")
    recreate.add_argument("--role", choices=ROLES, required=True)
    up = sub.add_parser("scale-up")
    up.add_argument("--role", choices=ROLES, required=True)
    down = sub.add_parser("drain")
    down.add_argument("--role", choices=ROLES, required=True)
    down.add_argument("--target", required=True)
    counts = sub.add_parser("scaled")
    counts.add_argument("--gateway", type=int, choices=(1, 2), required=True)
    counts.add_argument("--inference", type=int, choices=(1, 2), required=True)
    replace = sub.add_parser("rediscovery")
    replace.add_argument("--role", choices=ROLES, required=True)
    replace.add_argument("--old-ip", required=True)
    replace.add_argument("--old-instance", required=True)
    args = parser.parse_args()
    check_scope()
    client = Client()
    if args.action == "baseline":
        baseline(client)
    elif args.action == "scale-up":
        scale_up(client, args.role)
    elif args.action == "drain":
        drain(client, args.role, args.target)
    elif args.action == "scaled":
        scaled(client, args.gateway, args.inference)
    elif args.action == "rediscovery":
        rediscovery(client, args.role, args.old_ip, args.old_instance)
    elif args.action == "replace":
        replace_pool(client, args.role)
    elif args.action == "control-faults":
        control_faults(client)
    else:
        full(client)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        report("failed", error_type=type(error).__name__,
               detail=str(error) if isinstance(error, CheckError) else "예외 원문은 민감정보 보호를 위해 생략")
        sys.exit(1)
