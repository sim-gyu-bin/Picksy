# Picksy Wiki

Picksy의 현재 설계·제약·검증 과제를 모은 개발 지식이다. 공개 소개는 [README](../../README.md), 지식 관리 절차는 [프로젝트 스킬](../../.omp/skills/llm-wiki/SKILL.md)을 따른다.

## 주제

| 문서 | 내용 |
|---|---|
| [아키텍처](architecture.md) | Gateway·Decider·Caddy 구성, 네트워크·인증 경계 |
| [추론과 클라이언트 연동](inference.md) | Decider Q4 채택, 이전 모델 비교, OMP 계약, 품질·성능 한계 |
| [DB와 배포](operations.md) | SQLite, Migration, 이미지 배포, 복제·교체 |
| [로컬 개발 환경](local-development.md) | 실행 전제, 개발 포트·설정·데이터 분리 |
| [결정 이력](history/history.md) | 중요한 결정의 배경·이유·결과 |

## 초기 지식의 범위

단계 1 모델 선택은 Decider-2B Q4_K_M 채택으로 완료했고 단계 2 진행은 2026-11-13 사용자 승인됐다. 직접 CPU 라이브러리 추론과 HTTP 서비스·독립 풀 검증은 구분한다. 품질·지연 한계는 남아 있으며 단계 2 체크 항목과 OMP 종단 연동·ARM64·운영 배포는 검증 완료가 아니다. [채택 결정](history/2026-11-13-decider-q4-adoption.md).

현재 Wiki는 유효한 결론을, History는 선택 이유와 변경 과정을 담는다. 작업에 관련된 주제만 읽고, 구현 사실은 해당 코드·설정·테스트로 확인한다. 초기 설계 출처는 [인계된 설계 합의](history/2026-11-12-initial-design.md)다.

## 요구사항과 실행 계획

- [PRD](../prd.md): 제품 범위·요구사항·수용 조건의 초안. 새 세부 정책은 구현 전 검토하며 검증 완료를 뜻하지 않는다.
- [단계별 작업계획서](../plans/implementation-plan.md): 환경 확인부터 기술 검증·구현·배포·출시까지의 순서와 완료 조건. 한 단계씩 실행하고 결과를 확인한 뒤 다음 단계로 진행한다.
- [API 계약](../api-contract.md): 현재 확인한 Decider library 계약, OMP wire와 이전 Laya HTTP 계약의 구분.

요구사항은 PRD, 작업 체크 상태는 계획서, 검증된 현재 지식은 이 Wiki에서 관리한다.
