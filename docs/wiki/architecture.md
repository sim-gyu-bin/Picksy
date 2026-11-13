# 아키텍처

[Wiki](wiki.md)

## 현재 지식

**단계 3 Rust Gateway·DB 기반 완료 · d1 Q4 채택 · Rust+d1+Caddy 재통합 미검증.** 단계 4 진행은 승인됐지만 추론·OMP 연결은 아직 미구현이다. [DB 실행 근거](operations.md), [API 계약](../api-contract.md).

- 목표 구성은 Rust·Axum Gateway, d1-2B Q4_K_M 공식 llama.cpp 추론 서버, Caddy다.
- Gateway와 d1은 1:1 쌍이 아닌 독립 풀이다. 기본 각 1개이며 복제본 증가만으로 탐색·준비·분산 성공을 선언하지 않는다.
- 같은 Caddy의 공개 HTTPS 리스너는 Gateway 풀로, 비공개 내부 리스너는 d1 풀로 전달한다. 외부에는 공개 리스너만 노출하며 Gateway 인증 우회 경로를 만들지 않는다.
- Gateway는 인증·DB·키별 공유 제한 기반을 구현했다. 실제 추론 입력·모델 검증·응답·취소·자원 제한 연결은 단계 4 대상이다.
- 탐색·준비·분산 설계의 근거는 [단계 2 당시 Python 풀 실험](history/2026-11-13-pool-validation.md)이다. 이때 사용한 엔진과 제품 구성은 다르며 **d1의 실제 복제 성공 근거가 아니다**.

## 제약과 확인 필요 사항

- 표준 Caddy dynamic DNS upstream에는 active health check가 적용되지 않는다. [공식 제약](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy#dynamic-upstreams). 당시 탐색 관리기·관리 API 방식의 선택 이유와 실패 경계는 단계 2 이력에 보존한다.
- Rust+d1+Caddy에서 자동 탐색·준비 전 제외·분산·진행 요청 완료·축소·관리기 실패 차단을 다시 통합 검증해야 한다. 과거 실험의 주소·포트·시간 제한을 제품 확정값으로 간주하지 않는다.
- 단일 VM에서 서비스 복제는 VM 장애 대비가 아니다. Docker Desktop 결과는 Oracle ARM64·인터넷 경계·운영 배포 검증이 아니다.
- d1 단일 서버의 품질·지연 관측은 [추론 지식](inference.md)에 둔다. 두 복제본의 자원·처리량·운영 교체 자동화는 미검증이다.

## 근거와 관련 결정

- [d1 Q4 채택](history/2026-11-13-d1-q4-adoption.md)
- [단계 2 풀 실행 이력](history/2026-11-13-pool-validation.md)
- [최초 설계 이력](history/2026-11-12-initial-design.md)
- [README](../../README.md) · [DB와 배포](operations.md)
