# DB와 배포

[Wiki](wiki.md)

## 현재 지식

**설계 합의 · 미구현.** 아래는 운영 목표이며 실행 가능한 배포 절차가 아니다.

### DB와 Migration

- SQLite + SQLx를 사용하고 운영 DB는 Docker named volume에 영구 보관한다.
- 로컬 DB와 운영 DB를 분리한다. 로컬 DB 파일을 운영에 복사하지 않는다.
- Gateway를 두 개 실행할 경우 같은 VM의 DB 볼륨을 공유한다.
- AI가 Migration을 작성하고 검사한다. 승인된 배포에서 백업 → Migration 단일 실행 → 성공 시 서비스 배포 순서를 따른다.
- 신·구 코드가 공존할 수 있는 스키마 변경을 우선한다. 이미지 롤백은 DB 롤백이 아니다.

### 이미지와 배포

- GitHub Actions → ARM64 이미지 빌드 → GHCR → Oracle VM 배포를 목표로 한다.
- 운영 배포는 초기 수동 승인과 동시 배포 제한을 둔다.
- Intel Mac은 amd64 개발 환경, Oracle VM은 arm64 배포 환경이다.
- Docker MCP는 당장 도입하지 않고 CLI를 사용한다. 운영 DB 접근용 MCP는 필요할 때 읽기 전용으로 검토한다.

### 복제와 교체

- YAML 편집 없이 Gateway·Decider의 복제본 개수만 조절하는 것이 목표다. 구성 관계는 [아키텍처](architecture.md)에 둔다.
- Compose 자체에는 자동 롤링 업데이트가 없다. 요청 제외 → 드레이닝 → 교체 → 준비 확인 절차를 별도로 구성해야 한다.
- 실제 실행 명령·준비 조건·실패 처리 절차는 아직 구현되지 않았다. 복제본 증가만으로 무중단 배포가 보장되지 않는다.

## 제약과 확인 필요 사항

- Caddy의 복제본 자동 탐색·제거, 준비 상태 반영, 요청 분산을 실제 Compose 환경에서 검증해야 한다.
- Decider Q4 두 개의 메모리·CPU 사용량과 성능을 실측해야 한다. 단계 1 직접 CPU 실행은 HTTP 풀 검증이 아니다.
- Gateway 다중 프로세스의 SQLite 동시 접근·잠금 처리, Migration 단일 실행 보장, 백업 복구 절차를 구현·검증해야 한다.
- Docker Desktop 엔진 연결과 Compose 명령을 확인했고, 단계 1에서 격리된 Laya CPU 컨테이너를 실제 실행했다. 운영 Compose·Caddy 구성과 복제 검증은 아직 수행하지 않았다. 환경 경계는 [로컬 개발 환경](local-development.md), 실제 추론 결과는 [추론 Wiki](inference.md)에 둔다.

## 근거와 관련 결정

- [README](../../README.md): SQLite·Docker Compose·GitHub Actions 사용 방향.
- [인계된 설계 합의](history/2026-11-12-initial-design.md): 최초 구성의 역사적 근거.
- [Decider Q4 채택](history/2026-11-13-decider-q4-adoption.md): 단계 2 인프라 실험 승인과 운영 승인 분리. CI·Migration·운영 배포 검증은 아직 없다.
