# 단계 2 Python Gateway·Decider 독립 풀 검증

[결정 인덱스](history.md) · [현재 추론](../inference.md) · [현재 아키텍처](../architecture.md)

- 기록일: 2026-11-13
- 상태: 과거 실행·판단 보존. 현재 채택은 [d1 Q4](2026-11-13-d1-q4-adoption.md)다.
- 출처: 기존 문서와 실험 기록. 이번 문서 이동에서 추론·풀 검증을 재실행하지 않았다.

아래의 현재·보존·미승인 표현은 **각 실험 당시 상태**다. 후속 채택·정리 승인이 이를 대체하며, 로컬 원본 경로는 보존 위치가 아니라 삭제 승인된 당시 경로일 수 있다. 삭제 실행 완료를 주장하지 않는다.



**독립 풀 로컬 실험과 Rust Gateway·DB 기반 검증 완료 · 두 구성의 제품 통합 전.** 목표 구성은 Rust·Axum Gateway, Decider-2B Q4_K_M 추론 서버, Caddy다. 단계 2는 Python 실험 Gateway와 실제 Decider HTTP 래퍼를, 단계 3은 추론 없는 Rust Gateway·SQLite·키 관리를 각각 검증했다. [DB 기반 실행 근거](../operations.md). Rust와 Caddy·Decider 통합은 단계 4 대상이며 ARM64·운영 검증과 구분한다.

- Gateway와 Decider는 1:1 쌍이 아닌 독립적인 풀이다. 평소에는 각각 1개를 실행한다.
- 같은 Caddy의 공개 HTTPS 리스너는 Gateway 풀로, 비공개 내부 리스너는 Decider 풀로 요청을 전달한다.
- 외부에는 공개 리스너만 노출한다. Gateway 인증을 우회해 Decider에 접근할 수 없어야 한다.
- Gateway는 API 인증·요청 검증·호출 제한을 담당하는 방향이다. 구체적인 외부 API 및 오류 계약은 아직 확정하지 않았다.
- 단계 2의 내부 탐색 관리기는 Docker DNS로 후보를 찾고 준비 상태를 확인한 뒤 Caddy 관리 API에 사용 가능한 풀을 반영한다. Docker 소켓 없이 관리 API를 전용 네트워크로 격리했다. 공개 요청은 키·본문을 보내지 않는 관리망 `/permit` 검사도 거친다. 관리기 중단·동기화 실패에는 신규 요청을 차단하고, Caddy 실제 설정 ETag를 확인해 재시작 후 다시 반영한다.

표준 Caddy dynamic DNS upstream에는 active health check가 적용되지 않는다. 자동 탐색과 준비 전 제외를 함께 검증하기 위해 위 보조 기능을 선택했으며, 재시도로 미준비 요청 실패를 숨기지 않는다. [공식 제약](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy#dynamic-upstreams).

## 단계 2 실행 근거

`experiments/stage2/smoke.py full`을 Intel Mac의 Docker amd64에서 실행해 통과했다. 구성·재현 명령 (당시 경로: `experiments/stage2/README.md`).

- Gateway·추론 각각 YAML 변경 없이 실제 Compose 1→2→1, 두 풀 2+2에서 서로 다른 instance의 실제 응답을 관측했다.
- 준비 장벽에 있는 복제본은 제외됐고, 축소 시 기존 요청 완료·Caddy 반영·종료 코드 0을 확인했다. choice·noul·score 실제 추론과 응답 body 보존을 검사했다.
- 빈 풀은 503, controller 강제 종료 뒤 신규 요청은 502였으며 이미 접수된 요청은 완료됐다. Caddy 단독 재시작 후 worker 교체 없이 설정·TLS 연결이 복구됐다.
- 전체 풀 재생성은 계획 중단으로 검사했다. IP가 재사용되어 별도 endpoint 변경을 수행했고 Gateway `172.30.241.5→172.30.241.200`, 추론 `172.30.242.5→172.30.242.200`을 자동 재탐색했다. 자동 롤링 배포 성공으로 해석하지 않는다.
- 호스트 및 공개 ingress 전용 클라이언트에서 내부·관리 리스너 접근 차단, 인증 오류 401·추론 미호출을 확인했다. 이 범위는 Docker Desktop이며 원격 인터넷·Oracle 방화벽 검증은 아니다.
- 마지막 1+1 상태의 네 컨테이너에서 OOM 종료 없음·시험 키 로그 미노출을 확인했다. 이때 `docker stats`의 추론 메모리는 1.761 GiB였으며 최대 RSS나 2+2 부하 측정값은 아니다. 검증 후 승인된 단계 2 컨테이너·네트워크·임시 TLS 자원과 경계 진단 컨테이너만 정리했다. 기존 모델·결과·이미지는 보존했다.


## 제약과 확인 필요 사항

- 단일 VM 구성이다. 서비스 복제는 VM 자체의 장애 대비가 아니다.
- 내부 리스너·인증 경계와 자동 탐색·준비·분산은 위 로컬 실험에서 검증했다. 운영 환경에서도 별도로 검증해야 한다.
- YAML 편집 없이 복제본 개수만 조절하는 경로를 확인했으나, 요청 부하에 따른 성능 개선과 운영 교체 자동화는 아직 보장하지 않는다. [DB와 배포](../operations.md).
- 추론 서버와 OMP의 세부 계약·검증 과제는 [추론과 클라이언트 연동](../inference.md)에 둔다.

## 근거와 관련 결정

- [README 설계 방향](../../../README.md#설계-방향): 공개된 기술 선택 방향.
- [인계된 설계 합의](2026-11-12-initial-design.md): 최초 Laya 구성의 역사적 근거.
- [Decider Q4 채택](2026-11-13-decider-q4-adoption.md): 추론 엔진 교체와 단계 2 승인. 인증·독립 풀 경계는 유지한다.


## 당시 실험 절차 원본: `experiments/stage2/README.md`

아래 명령은 당시 실행 기록이며 현재 checkout의 실행 안내가 아니다.

# 단계 2: 독립 Gateway·Decider 풀

**로컬 실행 검증 완료.** Intel Mac의 Docker amd64에서 `python3 -B experiments/stage2/smoke.py full`을 실행해 통과했다. [관측 결과](../architecture.md). 제품 Rust Gateway, 운영 배포, OMP 종단 연동 또는 ARM64 성능 검증이 아니다. 모델 판단 품질과 인프라 성공을 구분한다.

## 구성과 승인된 선택

```text
호스트의 CA 검증 HTTPS 클라이언트
  → Caddy 공개 :443 (기본 호스트 127.0.0.1:18080)
  → 실험 Gateway 풀 :8080 (Bearer 인증)
  → 같은 Caddy의 gateways 전용 IP :18081
  → 원본 Decider-2B Q4_K_M 풀 :8080

내부 controller
  → Docker embedded DNS의 gateway/inference A 전체 조회
  → 각 복제본 :9090/status의 준비 상태 확인
  → management 전용 Caddy IP :2019/load에 정상 upstream만 반영
```

- Compose 기본 Gateway 1개·추론 1개다. 두 서비스에 `container_name`이나 고정 worker IP를 두지 않는다. controller·Caddy는 각각 하나다.
- 사용자 승인으로 **작은 탐색 controller + 표준 Caddy HTTP 관리 API**를 사용한다. 자체 DNS 서버, Docker 소켓, caddy-docker-proxy, 제품 코드 변경은 없다.
- `ingress`에는 Caddy만, `gateways`에는 Caddy·Gateway·controller, `inference`에는 Caddy·Decider·controller, `management`에는 Caddy·controller만 참여한다. 마지막 세 네트워크는 `internal: true`다.
- Caddy의 내부 리스너는 `gateways` IP에만, 관리 API는 `management` IP에만 bind한다. 단순히 포트를 publish하지 않는 것과 **공개 네트워크 인터페이스에서 리스너가 안 열리는 것**을 따로 검사한다. Gateway·추론·controller의 호스트 publish는 없다.
- Python `server.py`는 명시적인 실험 서버다. Gateway는 인증 키를 `hmac.compare_digest`로 확인한 뒤 고정 내부 Caddy에만 전달한다. 외부 Authorization·Host·임의 라우팅 헤더를 내부로 복사하지 않는다.
- 추론은 검증된 이미지의 `decider-ai==1.10.0`, `llama-cpp-python==0.3.36`을 검사하고 실제 GGUF SHA-256을 확인한 뒤 `Decider(...).system_one(...)`을 호출한다. 모델 다운로드, mock, 가중치·온도·`isolated_levels` 수정, 이름 보정은 없다. 동일 모델 인스턴스의 호출은 직렬화한다.
- 라이브러리 반환 JSON의 값은 변경하지 않는다. Gateway는 추론 HTTP 응답 body를 바이트 그대로 전달한다. `X-Stage2-Gateway`, `X-Stage2-Inference`, `X-Stage2-Request-ID`, `X-Stage2-Body-SHA256`으로 인프라만 관측한다. 모델명·확률·score·confidence에 인프라 메타데이터를 섞지 않는다. config의 기존 `decider-2b-v11` 표시도 임의 수정하지 않는다.

## 준비·종료·오류 정책

1. worker 제어면 `:9090`은 로딩 중에도 상태만 반환한다. 실제 데이터 리스너 `:8080`은 모델 로딩·해시·버전 검사가 끝나고 입장 장벽이 풀린 뒤에만 연다.
2. controller는 0.25초 간격으로 Docker DNS의 모든 IPv4 주소와 준비 상태를 조회한다. probe timeout은 0.5초, 관리 API timeout은 3초다. 매번 실제 `/config/` ETag도 대조하며 정상 주소 목록이나 실제 설정이 바뀌면 원자적인 `POST /load`로 갱신한다. Caddy만 재시작해 bootstrap으로 돌아가도 재동기화한다. 초기 설정은 양쪽 풀 모두 명시적인 503이다.
3. **DNS 조회 실패는 이전 목록 유지가 아니라 해당 풀 503**, probe 실패/미준비는 해당 주소 제외다. 대체 upstream·모델·HTTP 재시도는 없다. Caddy `lb_retries=0`, upstream keepalive 비활성, Python HTTP 클라이언트도 재시도하지 않는다. 준비 상태 polling은 추론 재시도가 아니다.
4. SIGTERM을 받으면 worker는 `withdrawing`이 되어 준비 목록에서 먼저 빠진다. controller의 DNS/probe/API 전파를 위한 8초 동안 이미 배정된 요청을 받을 수 있고, 이후 `draining`으로 신규 접수를 닫아 기접수 요청과 대기 중 모델 호출이 끝날 때까지 기다린다. Compose 종료 유예는 180초다. smoke는 단순 sleep 성공이 아니라 `applied_a`에서 실제 제외된 세대 증가를 먼저 관측하고 이후 새 요청이 생존자에게만 가는지 확인한다.
5. 시험의 `hold-completion`은 실제 추론을 실행한 뒤 HTTP 응답 완료만 잠시 막는 장벽이다. `active` 상태를 보고 실제 SIGTERM/Compose 축소를 시작한 뒤 장벽을 해제한다. 단순 응답 mock·추론 생략·고정 sleep으로 요청을 꾸미지 않는다. 이 장벽을 쓴 결과는 지연/처리량 측정이 아니다. 장벽은 최대 120초다.
6. Caddy reload 중 기존 요청 완료는 공식 사양과 별도로 실제 응답으로 확인한다. **`--force-recreate`를 자동 rolling update라고 부르지 않는다.** 전체 풀 교체 시험은 두 복제본을 정상 stop하고 503을 확인한 계획 중단이다. 진행 요청 보존은 별도의 2→1 축소에서 검사한다.
7. 공개 Caddy 입구는 표준 `forward_auth`로 관리망 controller `GET /permit`을 **한 번만** 확인한다. 사용자 인증은 여전히 Gateway에서 한다. 원본 요청 헤더 전체, Authorization, Cookie, forward_auth가 추가하는 원본 URI/메서드까지 제거하고 고정 Host만 지정한다. GET이므로 본문을 전달/소비하지 않는다. 확인 성공 후 들어간 요청에는 내부 리스너에서 다시 gate를 걸지 않아 controller 장애 뒤에도 이미 접수된 요청을 완료할 수 있다.
8. `/permit`은 마지막 정상 reconciliation 이후 2초 이내이고 관리 API 오류가 없을 때만 200이다. 관리 API 실패·controller 정지·SIGKILL/OOM·loop 정체는 신규 외부 요청을 차단한다. 응답 가능한 비준비 상태는 503, 연결 불가/timeout은 Caddy의 502가 될 수 있으며 성공 응답으로 대체하지 않는다. `finally`의 빈 설정 전송에 안전성을 의존하지 않는다. 제어면은 정상 실제 설정을 재관측할 때까지 닫힌 상태로 재동기화하며 사용자 추론을 재시도하지 않는다.

## 자원과 비밀값

- 기존 런타임: `sha256:c6632d172bb586d377bfebd7625348f1daf981818f325338c125f92aeab9f41f`. 새 빌드를 하지 않고 이 디렉터리만 읽기 전용 마운트한다. 모든 Python 서비스는 Linux amd64다.
- Caddy: `2.10.2-alpine@sha256:4c6e91c6ed0e2fa03efd5b44747b625fec79bc9cd06ac5235a779726618e530d`. 메인이 pull한 digest다. 실제 실행 버전·모듈은 smoke에서도 확인한다.
- 모델 경로는 기존 `.local/picksy-dev/decider-q8/model`이며 Q4 파일과 원본 tokenizer/config를 그대로 읽는다. Q4 파일 SHA-256은 `e6d3a7534a3abc3a723f8263dc694b86f533214a5bf761474113c66d44018340`이다. 실험 소스 이름과 로컬 보존 경로는 별개다.
- 추론 하나당 CPU 4개분·메모리 3 GiB·추가 swap 없음. 기존 단일 Q4 RSS 약 2.4 GiB를 바탕으로 두 추론의 실제 RSS 약 5 GiB를 예상한 **실험 예산**이며 보장이 아니다. 두 추론을 넘겨 동시에 기동하지 않는다. Caddy·Gateway·controller의 메모리 상한은 각각 128 MiB다. Docker VM 7.75 GiB에서 실제 OOM 여부는 메인이 관측한다.
- 외부 API 키는 실행 때 생성한 환경변수만 사용한다. `.env`, 키 파일, 저장소, 보고서에 저장하거나 출력하지 않는다. Docker 권한자는 컨테이너 환경을 읽을 수 있으므로 이 실험이 호스트 관리자에게 키를 숨기는 수단은 아니다. `set -x`, 전체 `docker inspect`, `docker compose config` 출력은 금지한다.
- Caddy가 생성하는 TLS 개인키는 컨테이너 `/data` tmpfs에만 있고 호스트/영속 볼륨에 저장하지 않는다. smoke는 **공개 CA 인증서만** `docker exec ... cat .../root.crt`로 메모리에 읽어 정상 인증서·호스트명 검증을 한다. `-k` 또는 `CERT_NONE`은 없다. Caddy 재시작 후에는 새 CA를 다시 읽는다.
- HTTP 접근 로그를 켜지 않고 오류 로그의 헤더·URI도 제거한다. 실험 Python 로그에는 입력·응답·인증 키·예외 원문을 남기지 않는다. smoke도 원본 body 없이 상태·instance·주소·종료 결과만 출력한다.

## 메인 실행 절차

저장소 루트에서 같은 셸 환경으로 실행한다. 기존 18080/18081 충돌과 Docker VM 자원은 메인이 확인했다. 네트워크 대역이 충돌하면 시작 전에 `STAGE2_GATEWAY_PREFIX`, `STAGE2_INFERENCE_PREFIX`, `STAGE2_MANAGEMENT_PREFIX`를 서로 다른 미사용 `/24`의 앞 세 옥텟으로 지정한다. 기본은 각각 `172.30.241`, `172.30.242`, `172.30.243`이다.

```sh
# 비밀값은 출력하지 않는다. ~/.env 등의 파일로 저장하지 않는다.
export STAGE2_API_KEY="$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')"
export STAGE2_MODEL_DIR="$PWD/.local/picksy-dev/decider-q8/model"
unset STAGE2_START_HELD

# 설정 구문만 검사한다. -q 없이 환경을 펼쳐 출력하지 않는다.
docker compose -p picksy-stage2 -f experiments/stage2/compose.yaml config -q

# 최초 1+1 기동. 기존 단계 1 자원·모델·이미지에는 손대지 않는다.
docker compose -p picksy-stage2 -f experiments/stage2/compose.yaml up -d

# 실제 전체 시나리오. CA 생성·worker 준비·controller applied 목록은 제한 시간 내 기다린다.
python3 -B experiments/stage2/smoke.py full

# 성공 후 최종 1+1 상태에서 필요할 때만 메타데이터 확인.
docker compose -p picksy-stage2 -f experiments/stage2/compose.yaml ps

# 기본 종료는 stop이며 결과 관측 후 정리한다.
docker compose -p picksy-stage2 -f experiments/stage2/compose.yaml stop --timeout 180
# 메인이 로그/종료 결과를 확인한 뒤 승인된 stage2 프로젝트만 정리할 때:
docker compose -p picksy-stage2 -f experiments/stage2/compose.yaml down --volumes
unset STAGE2_API_KEY
```

메인에게 승인된 별도 정리 범위는 **이번 단계 2가 만든 컨테이너·네트워크·임시 인증서 저장 자원만**이다. 메인은 관측을 마친 후 이 프로젝트에 `down --volumes`를 적용할 수 있다. 기존 `.local` 모델·결과, 이전 실험 이미지·컨테이너·볼륨을 일반적인 `prune`이나 광역 삭제로 정리하지 않는다. `full`은 자동 `down`/`prune`을 하지 않고 마지막 1+1을 실행 상태로 남긴다. 공개망 진단 클라이언트는 `picksy-stage2-boundary-*`, `io.picksy.stage2=true` 라벨의 정상 종료 컨테이너로 남으며 메인의 승인된 단계 2 정리 대상이다.

`full`이 실제 수행하는 좁은 시나리오:

1. Caddy 버전/표준 A 모듈 존재, 미신뢰 CA 거부, 올바른 CA 검증, 인증 누락·오류 401 및 모델 미호출, 잘못된 JSON 400.
2. 고정 합성 질문 choice·noul·score 각 1건의 실제 응답 구조·확률 범위/합·score 기대값·원본 byte 전달 검사. 품질 통과선을 바꾸거나 정답률을 주장하지 않는다.
3. 호스트에서 내부 IP/포트 접근 차단, 공개 ingress에만 붙는 별도 클라이언트에서 공개 Caddy IP의 443 접근 가능 및 18081/2019 차단. Gateway/추론망에서 관리 API 접근 불가도 따로 검사한다. 다른 OS·인터넷 원격 클라이언트 검증으로 일반화하지 않는다.
   초기 1+1에서 controller를 실제 SIGKILL해 신규 요청 차단과 이미 접수된 실제 추론 완료를 확인한다. controller 복구 후 Caddy만 재시작해 실제 설정 재동기화·worker ID 불변·현재 CA 재취득과 TLS 성공을 확인한다. CA 변경 여부는 관측값으로 출력한다.
4. Gateway만 `--scale gateway=2`로 확장: 새 복제본을 명시적인 준비 장벽에 둔 채 기존 서버로 실제 요청, 장벽 해제 뒤 instance 헤더의 분산, 반대 풀 ID 불변.
5. 실제 진행 요청을 유지한 채 `--scale gateway=1` 수행: controller 제외·Caddy reload·생존자 신규 처리·기존 응답 완료·실제 종료 코드 0·Compose 총 개수 1. stop만 한 상태를 scale-down으로 세지 않는다.
6. 추론만 같은 1→2→1, 이후 두 풀 모두 2+2로 독립 확장/분산.
7. 각 풀의 정상 stop→빈 풀/Docker DNS 조회 실패→HTTP 503→`--force-recreate`로 두 프로세스 교체. 기존 컨테이너 ID와 새 ID를 구분한다. Docker가 같은 IP를 재사용하면 **새 복제본 하나의 승인된 네트워크 endpoint를 미사용 IP로 재연결한 별도 사건**으로 기록한다. IP 점유용 컨테이너나 별도 모델 one-off를 만들지 않는다. 실제 새 IP의 Docker DNS·controller 반영과 분산을 다시 확인한다.
8. 변경된 네트워크 경계 재확인, 각 풀을 실제 2→1로 줄여 마지막 1+1의 분산/종료 상태 확인.

모든 추론 HTTP 요청은 한 번만 보낸다. 첫 실패에서 nonzero로 끝내며 확인되지 않은 단계는 통과로 출력하지 않는다. 처음부터 `full`을 반복해 실패를 지우지 말고, 실패 원인 수정 후 해당 단계의 좁은 명령을 사용한다. `smoke.py`는 프로젝트와 `io.picksy.stage2=true` 라벨을 확인한 뒤 변경한다.

### 실패 지점의 작은 실행

```sh
python3 -B experiments/stage2/smoke.py baseline
python3 -B experiments/stage2/smoke.py control-faults
python3 -B experiments/stage2/smoke.py scale-up --role gateway
python3 -B experiments/stage2/smoke.py scaled --gateway 2 --inference 1

# --target은 해당 서비스의 Compose container-number=2인 실제 ID다.
# 키가 포함된 전체 inspect 대신 단일 라벨만 조회한다.
docker inspect --format '{{index .Config.Labels "com.docker.compose.container-number"}}' "$TARGET"
python3 -B experiments/stage2/smoke.py drain --role gateway --target "$TARGET"
python3 -B experiments/stage2/smoke.py scaled --gateway 1 --inference 1

# 해당 풀 두 개가 준비된 상태에서 계획 중단/재생성/IP 재탐색만 확인한다.
python3 -B experiments/stage2/smoke.py replace --role inference
```

수동으로 개수만 바꾸는 공식 진입점도 같은 Compose다. 2→1 명령은 두 번째 복제본을 실제 종료·삭제하므로 이 단계 2 승인 범위에만 실행한다.

```sh
docker compose -p picksy-stage2 -f experiments/stage2/compose.yaml up -d --no-deps --no-recreate --scale gateway=2 gateway
docker compose -p picksy-stage2 -f experiments/stage2/compose.yaml up -d --no-deps --no-recreate --scale inference=2 inference
docker compose -p picksy-stage2 -f experiments/stage2/compose.yaml up -d --no-deps --no-recreate --scale gateway=1 gateway
docker compose -p picksy-stage2 -f experiments/stage2/compose.yaml up -d --no-deps --no-recreate --scale inference=1 inference
```

## 공식 근거와 미검증 경계

- [Caddy reverse_proxy / dynamic upstreams](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy#dynamic-upstreams): 표준 A/AAAA 탐색은 있지만 **dynamic upstream에 active health check는 실행되지 않는다**. Docker DNS가 반환한 복제본의 준비 여부까지 보장하지 않는다.
- [Caddy v2.10.2 AUpstreams 소스](https://github.com/caddyserver/caddy/blob/v2.10.2/modules/caddyhttp/reverseproxy/upstreams.go): `http.reverse_proxy.upstreams.a` 표준 등록·주소별 목록·refresh cache 구현. smoke의 모듈 존재 확인은 그 모듈로 준비 상태 라우팅을 실행했다는 뜻이 아니다. 승인된 최종 구성은 controller가 **static upstream**을 갱신한다.
- [Caddy API](https://caddyserver.com/docs/api#post-load): `POST /load`의 원자적인 설정 반영, graceful reload, 실패 시 기존 설정 유지. 후자는 controller가 빈 풀 정책과 관리 API 실패를 구분해야 하는 이유다.
- [Caddy forward_auth](https://caddyserver.com/docs/caddyfile/directives/forward_auth): 원본 body를 소비하지 않는 GET 사전 요청 및 2xx에서만 다음 handler 진행. 기본으로 원본 URI/메서드를 헤더에 넣으므로 그대로 사용하지 않는다. [v2.10.2 HeaderOps 소스](https://github.com/caddyserver/caddy/blob/v2.10.2/modules/caddyhttp/headers/headers.go)의 전체 헤더 삭제→set→개별/와일드카드 삭제 순서에 따라 `-*` 이후 `-X-Forwarded-*`까지 지정한다. Gateway 인증 키와 원문 URI가 permit에 전달되지 않아야 실제 검사도 통과한다.
- [Caddy v2.10.2 HTTP Stop 소스](https://github.com/caddyserver/caddy/blob/v2.10.2/modules/caddyhttp/app.go): reload는 새 접수를 이전 서버에서 중지하고 진행 연결 완료는 비동기로 기다린다. 프로세스 종료와 달리 reload에서 완료 전체를 blocking하지 않는다. 실제 응답 완료 관측은 smoke가 담당한다.
- [Caddy bind](https://caddyserver.com/docs/caddyfile/directives/bind): 특정 인터페이스/IP 리스너의 의미. 관리 API도 별도 주소에 제한한다.
- [Docker Compose 네트워킹](https://docs.docker.com/compose/how-tos/networking/), [Docker embedded DNS](https://docs.docker.com/engine/network/#dns-services): 서비스 이름 탐색·재생성 시 IP 변화 가능성·`127.0.0.11`·사용자 정의 네트워크. Docker DNS IP 변경 가능성과 실제 다른 IP 관측은 구분한다.
- [Compose scale](https://docs.docker.com/reference/cli/docker/compose/scale/), [Compose up](https://docs.docker.com/reference/cli/docker/compose/up/): 복제본 증감/재생성은 준비 상태 기반 rolling update를 보장하지 않는다.
- 대안 검토: [caddy-docker-proxy 공식 README](https://github.com/lucaslorentz/caddy-docker-proxy), 고정 소스 [generator.go](https://github.com/lucaslorentz/caddy-docker-proxy/blob/c78444b2b09afc9393b06922b1a6231b407cdeb7/generator/generator.go)·[containers.go](https://github.com/lucaslorentz/caddy-docker-proxy/blob/c78444b2b09afc9393b06922b1a6231b407cdeb7/generator/containers.go). 표준 Caddy 내장 기능이 아닌 Docker 메타데이터 기반 plugin이며 해당 컨테이너 선택 경로에 health 필터가 없다. Docker socket 권한 추가 없이 controller 방식을 채택했다.

공식 자료 확인은 실제 연동 성공이 아니다. 실행 결과·사용한 환경·실패·모델 품질 제한은 메인이 [아키텍처 Wiki](../architecture.md)와 [추론 Wiki](../inference.md)에 갱신한다. 제품 채택 이후에도 이 실험의 합성 질문 몇 건을 Jev 동급 품질이나 Oracle ARM 성능 근거로 사용하지 않는다.


## 당시 계획서 완료 기록



- `python3 -B experiments/stage2/smoke.py full` 성공. Caddy 2.10.2, 실제 Decider Q4, 인증·TLS·응답 원문 보존, 양쪽 독립 1→2→1 및 2+2 요청 분산을 관측했다.
- 미준비 복제본 제외, 실제 진행 요청 완료 후 축소·종료 코드 0, 빈 풀 503, controller SIGKILL 뒤 신규 요청 502·기접수 응답 완료, Caddy 재시작 후 설정 재동기화를 확인했다.
- 풀 전체 교체는 계획 중단이며 자동 롤링 배포가 아니다. Docker의 IP 재사용 때문에 별도로 endpoint IP를 변경해 재탐색을 확인했다.
- 외부 차단은 현재 Docker Desktop 호스트와 ingress 전용 클라이언트에서 검증했다. Oracle ARM·인터넷 원격 경계·제품 Rust Gateway·DB·OMP 연동·두 복제본의 성능 비교는 미검증이다.
- 재현 구성과 근거: 단계 2 실험 (당시 경로: `experiments/stage2/README.md`), [현재 아키텍처](../architecture.md).


