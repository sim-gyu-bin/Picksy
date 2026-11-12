# 아키텍처

[Wiki](wiki.md)

## 현재 지식

**독립 풀의 로컬 실행 검증 완료 · 제품 구현 전.** 목표 구성은 Rust·Axum Gateway, Decider-2B Q4_K_M 추론 서버, Caddy다. 단계 2에서는 명시적인 Python 실험 Gateway와 실제 Decider HTTP 래퍼를 사용했다. Rust 제품 구현·ARM64·운영 검증과 구분한다.

- Gateway와 Decider는 1:1 쌍이 아닌 독립적인 풀이다. 평소에는 각각 1개를 실행한다.
- 같은 Caddy의 공개 HTTPS 리스너는 Gateway 풀로, 비공개 내부 리스너는 Decider 풀로 요청을 전달한다.
- 외부에는 공개 리스너만 노출한다. Gateway 인증을 우회해 Decider에 접근할 수 없어야 한다.
- Gateway는 API 인증·요청 검증·호출 제한을 담당하는 방향이다. 구체적인 외부 API 및 오류 계약은 아직 확정하지 않았다.
- 단계 2의 내부 탐색 관리기는 Docker DNS로 후보를 찾고 준비 상태를 확인한 뒤 Caddy 관리 API에 사용 가능한 풀을 반영한다. Docker 소켓 없이 관리 API를 전용 네트워크로 격리했다. 공개 요청은 키·본문을 보내지 않는 관리망 `/permit` 검사도 거친다. 관리기 중단·동기화 실패에는 신규 요청을 차단하고, Caddy 실제 설정 ETag를 확인해 재시작 후 다시 반영한다.

표준 Caddy dynamic DNS upstream에는 active health check가 적용되지 않는다. 자동 탐색과 준비 전 제외를 함께 검증하기 위해 위 보조 기능을 선택했으며, 재시도로 미준비 요청 실패를 숨기지 않는다. [공식 제약](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy#dynamic-upstreams).

## 단계 2 실행 근거

`experiments/stage2/smoke.py full`을 Intel Mac의 Docker amd64에서 실행해 통과했다. [구성·재현 명령](../../experiments/stage2/README.md).

- Gateway·추론 각각 YAML 변경 없이 실제 Compose 1→2→1, 두 풀 2+2에서 서로 다른 instance의 실제 응답을 관측했다.
- 준비 장벽에 있는 복제본은 제외됐고, 축소 시 기존 요청 완료·Caddy 반영·종료 코드 0을 확인했다. choice·noul·score 실제 추론과 응답 body 보존을 검사했다.
- 빈 풀은 503, controller 강제 종료 뒤 신규 요청은 502였으며 이미 접수된 요청은 완료됐다. Caddy 단독 재시작 후 worker 교체 없이 설정·TLS 연결이 복구됐다.
- 전체 풀 재생성은 계획 중단으로 검사했다. IP가 재사용되어 별도 endpoint 변경을 수행했고 Gateway `172.30.241.5→172.30.241.200`, 추론 `172.30.242.5→172.30.242.200`을 자동 재탐색했다. 자동 롤링 배포 성공으로 해석하지 않는다.
- 호스트 및 공개 ingress 전용 클라이언트에서 내부·관리 리스너 접근 차단, 인증 오류 401·추론 미호출을 확인했다. 이 범위는 Docker Desktop이며 원격 인터넷·Oracle 방화벽 검증은 아니다.
- 마지막 1+1 상태의 네 컨테이너에서 OOM 종료 없음·시험 키 로그 미노출을 확인했다. 이때 `docker stats`의 추론 메모리는 1.761 GiB였으며 최대 RSS나 2+2 부하 측정값은 아니다. 검증 후 승인된 단계 2 컨테이너·네트워크·임시 TLS 자원과 경계 진단 컨테이너만 정리했다. 기존 모델·결과·이미지는 보존했다.


## 제약과 확인 필요 사항

- 단일 VM 구성이다. 서비스 복제는 VM 자체의 장애 대비가 아니다.
- 내부 리스너·인증 경계와 자동 탐색·준비·분산은 위 로컬 실험에서 검증했다. 운영 환경에서도 별도로 검증해야 한다.
- YAML 편집 없이 복제본 개수만 조절하는 경로를 확인했으나, 요청 부하에 따른 성능 개선과 운영 교체 자동화는 아직 보장하지 않는다. [DB와 배포](operations.md).
- 추론 서버와 OMP의 세부 계약·검증 과제는 [추론과 클라이언트 연동](inference.md)에 둔다.

## 근거와 관련 결정

- [README 설계 방향](../../README.md#설계-방향): 공개된 기술 선택 방향.
- [인계된 설계 합의](history/2026-11-12-initial-design.md): 최초 Laya 구성의 역사적 근거.
- [Decider Q4 채택](history/2026-11-13-decider-q4-adoption.md): 추론 엔진 교체와 단계 2 승인. 인증·독립 풀 경계는 유지한다.
