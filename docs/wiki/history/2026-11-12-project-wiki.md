# 프로젝트 전용 Wiki 채택

[결정 인덱스](history.md) · [현재 Wiki](../wiki.md) · [운영 규칙](../../../.omp/skills/llm-wiki/SKILL.md)

- 기록일: 2026-11-12
- 결정일: 2026-11-12
- 상태: 합의
- 출처: 프로젝트 전용 스킬과 초기 Wiki 구조 제안에 대한 사용자의 생성 요청.

## 배경과 결정

개발·기술 검토에서 재사용할 지식을 저장소 안에 유지한다. `.omp/skills/llm-wiki/SKILL.md`에 Query / Ingest / Lint와 문서 양식을 두고, `docs/wiki/`에는 현재 지식, `history/`에는 중요한 결정의 배경과 결과를 둔다.

## 대안과 선택 이유

- [Notion 제안](https://app.notion.com/p/LLM-Wiki-Skill-410988581540813f9755c4b4ce23cfb0)의 전역 스킬 대신 프로젝트 스킬을 선택했다. 이번 요청의 범위를 Picksy로 한정하고 다른 프로젝트에 영향을 주지 않는다.
- [Mycelium 운영 규칙](https://github.com/RichardTathata/mycelium/blob/main/docs/wiki/AGENTS.md)에서 선택적 조회, 원본 근거 연결, 중복 방지 원칙을 채택했다. Rust 라이브러리 전용 분류와 깊은 계층·별도 ingest 로그는 초기 규모에 불필요해 도입하지 않는다.
- Life-Bridge 전용 경로·규칙을 복사하지 않는다. Git + Markdown으로 운영하며 외부 MCP·데몬·패키지를 추가하지 않는다.

## 결과·단점·검증 상태

문서는 작은 주제별 구조로 시작하고 내용이 생길 때 확장한다. 사실의 신선도는 원본 대조에 의존하므로 Wiki를 읽는 것만으로 구현 검증을 대신할 수 없다. 스킬 파일 설치만으로 매 작업 자동 호출이 보장되지는 않는다.

초기 Wiki는 사용자 인계와 README에 근거하며 애플리케이션 실행·배포 검증 결과를 제공하지 않는다.
