# Picksy Wiki

Picksy의 현재 설계·제약·검증 과제를 모은 개발 지식이다. 공개 소개는 [README](../../README.md), 지식 관리 절차는 [프로젝트 스킬](../../.omp/skills/llm-wiki/SKILL.md)을 따른다.

## 주제

| 문서 | 내용 |
|---|---|
| [아키텍처](architecture.md) | Gateway·d1·Caddy 구성, 네트워크·인증 경계 |
| [추론과 클라이언트 연동](inference.md) | d1 Q4 현재 계약·실측, OMP 계약, 품질·성능 한계 |
| [DB와 배포](operations.md) | 단계 4·4A·4B 실행 근거, 관리자 세션·공유 제한·Migration과 운영 경계 |
| [로컬 개발 환경](local-development.md) | 실행 전제, 개발 포트·설정·데이터 분리 |
| [결정 이력](history/history.md) | 중요한 결정의 배경·이유·결과 |

## 초기 지식의 범위

단계 0~4는 완료했다. 현재 모델은 [d1 Q4 채택 결정](history/2026-11-13-d1-q4-adoption.md)을 따른다. 단계 2의 [과거 Python 풀 실행 결과](history/2026-11-13-pool-validation.md), 단계 3의 [키·DB 실행 결과](operations.md), 단계 4의 [실제 OMP→Rust Gateway→d1 검증](../plans/implementation-plan.md#단계-4-결과)은 별도 근거다. 단계 5는 미착수이며 Rust+d1+Caddy 제품 풀·d1 복제·Oracle ARM64·운영 배포는 미검증이다. OMP 역할 자동 선택·ChainJudge fallback도 이번 검증에 포함하지 않았다. 이전 모델 비교·실패·계약은 [비교 이력](history/2026-11-13-inference-comparison.md)에 보존한다.

후속 합의는 **Picksy Server가 관리자 화면·관리 API·추론 API를 함께 제공하는 모노레포**다. **4A 공통 데이터, 4B Google 로그인·관리 HTTP, 4C React 화면·서버 이미지의 구현과 로컬 검증을 완료했다.** [API 계약](../api-contract.md), [데이터·실행 근거](operations.md), [로컬 개발·watch·이미지 실행](local-development.md)을 참조한다. E1 준비와 호스트 Rust의 실제 Google 화면 흐름은 완료했으며, amd64 컨테이너에서는 정적 제공·API 설정/인증 경계를 검증했다. 컨테이너의 실제 Google 로그인·4D 종합 수용·제품 풀·ARM64·운영 배포는 별도 범위다. Oracle·운영 준비는 [계획서](../plans/implementation-plan.md)의 E2~E3로 분리하며 로컬 결과가 기존 DB 변경이나 운영 배포를 승인하지 않는다.

현재 Wiki는 유효한 결론을, History는 선택 이유와 변경 과정을 담는다. 작업에 관련된 주제만 읽고, 구현 사실은 해당 코드·설정·테스트로 확인한다. 초기 설계 출처는 [인계된 설계 합의](history/2026-11-12-initial-design.md)다.

## 요구사항과 실행 계획

- [PRD](../prd.md): 제품 범위·요구사항·수용 조건의 초안. 새 세부 정책은 구현 전 검토하며 검증 완료를 뜻하지 않는다.
- [단계별 작업계획서](../plans/implementation-plan.md): 환경 확인부터 기술 검증·구현·배포·출시까지의 순서와 완료 조건. 한 단계씩 실행하고 결과를 확인한 뒤 다음 단계로 진행한다.
- [API 계약](../api-contract.md): 구현·로컬 검증된 추론·4A 공통 키·사용량·4B 관리자 인증·관리 HTTP 계약과 후속 검증 경계.

요구사항은 PRD, 작업 체크 상태는 계획서, 검증된 현재 지식은 이 Wiki에서 관리한다.
