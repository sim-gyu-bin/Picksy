# DB와 배포

[Wiki](wiki.md)

## 현재 지식

**단계 3 로컬 Gateway·DB 기반 검증 완료 · 운영 배포 미구현.** 과거 구성의 단계 2 풀 실험과 단계 3 Rust 직접 실행은 완료했다. 현재 d1 Q4 채택·단계 4 진행은 승인됐지만 추론 연결은 미구현이며 Rust+d1+Caddy 재통합·CI 원격 실행·운영 백업/복구·배포는 미검증이다.

### DB와 Migration

- SQLite + SQLx를 사용하고 운영 DB는 Docker named volume에 영구 보관한다.
- 로컬 DB와 운영 DB를 분리한다. 로컬 DB 파일을 운영에 복사하지 않는다.
- Gateway를 두 개 실행할 경우 같은 VM의 DB 볼륨을 공유한다.
- AI가 Migration을 작성하고 검사한다. 승인된 배포에서 백업 → Migration 단일 실행 → 성공 시 서비스 배포 순서를 따른다.
- 신·구 코드가 공존할 수 있는 스키마 변경을 우선한다. 이미지 롤백은 DB 롤백이 아니다.

### 구현된 키·사용량·제한

- [초기 Migration](../../migrations/0001_gateway.sql)은 키 해시·키별 현재 분 카운터·요청 메타데이터를 저장한다. [DB 코드](../../src/db.rs)의 `migrate`만 생성·Migration하며 `Store::open`은 기존 스키마를 검사한다. 서버 기동·키 명령이 자동 Migration하지 않는다.
- SQLite는 WAL, `synchronous=FULL`, 외래 키 활성화, 쓰기/풀 획득 대기 5초, 프로세스당 최대 연결 4개다. 운영의 승인된 백업과 Migration 단일 실행 절차를 대체하지 않는다.
- 로컬 CLI는 키 발급·목록·폐기·사용량을 제공한다. 키는 `pk_` + OS 난수 256비트의 16진수이며 원문을 한 번만 출력한다. 저장 검증값은 SHA-256이고 목록에 해시도 노출하지 않는다. [정책 선택과 이유](history/2026-11-13-gateway-key-policy.md), [실행 명령](local-development.md).
- `Store::admit`은 `BEGIN IMMEDIATE` 안에서 유효 키·폐기 여부·고정 UTC 분 한도를 검사하고 카운터와 사용량 접수를 함께 커밋한다. 기본 한도는 키별 서비스 전체 60회/분이다. 각 Gateway에 별도 DB를 연결하면 공유 제한이 아니므로 같은 로컬 파일/볼륨을 사용해야 한다.
- 인증 실패 401은 DB 사용량에서 제외한다. 인증 후 한도 거절 429는 사용량에 포함하되 카운터를 추가 차감하지 않는다. 허용 요청은 pending으로 접수하고 응답 전에 완료 상태·소요 시간을 기록한다. 완료 쓰기 실패는 503이며 pending을 성공으로 집계하지 않는다.
- 집계는 `total = pending + succeeded + rejected + failed + other_completed`다. `rejected`는 기록된 4xx이며, 접수 전 DB 장애와 인증 실패는 포함하지 않는다. HTTP 전달 성공과 DB의 완료 상태는 동일하지 않다. 자동 삭제·자동 보존 만료는 없으며 사용량 데이터는 누적된다.
- [HTTP 미들웨어](../../src/gateway.rs)는 매 요청 DB 검사로 폐기를 반영하고 원문 Authorization을 후속 핸들러에 넘기지 않는다. 요청 ID·상태·소요 시간만 로그에 남긴다. 추론 경로·취소·대기열 제한은 단계 4 대상이다.

### 단계 3 실행 근거

- Intel Mac `x86_64-apple-darwin`, Rust 1.95.0에서 `bash scripts/check.sh`의 포맷 확인·Clippy 경고 거부·통합 테스트 9개와 `cargo build --locked --bin picksy`를 통과했다. [CI](../../.github/workflows/check.yml)는 같은 검사 진입점을 사용하지만 원격 Actions 실행은 아직 확인하지 않았다.
- 실제 CLI로 Migration·키 생성·목록·폐기·사용량을 실행하고, 두 독립 서버 프로세스에서 누락/잘못된/중복 인증 401, 유효 키 204, 폐기 및 서버 재시작 뒤 401을 확인했다.
- 한도 5인 키로 두 프로세스에 동시 12건을 보내 총 5건 204·7건 429를 관측했다. SQLite 쓰기 잠금을 잡으면 503, 해제 후 204였고 한도를 우회하지 않았다.
- 실제 집계는 성공 8·거절 7·pending 0·전체 15였다. DB·WAL·로그의 원문 키 미노출을 확인했다. 완료 기록 실패·접수 롤백·429 기록 실패·DB 오류·명시적 Migration 경계는 격리 통합 테스트로 확인했다.
- 검증 서버를 정상 종료했고 데이터는 `.local/picksy-dev/stage3-667b8a9c95/` 및 테스트별 고유 폴더에 보존했다. 운영 DB·Rust+d1+Caddy 통합·OMP·ARM64는 이 검증에 포함하지 않았다.


### 이미지와 배포

- GitHub Actions → ARM64 이미지 빌드 → GHCR → Oracle VM 배포를 목표로 한다.
- 운영 배포는 초기 수동 승인과 동시 배포 제한을 둔다.
- Intel Mac은 amd64 개발 환경, Oracle VM은 arm64 배포 환경이다.
- Docker MCP는 당장 도입하지 않고 CLI를 사용한다. 운영 DB 접근용 MCP는 필요할 때 읽기 전용으로 검토한다.

### 복제와 교체

- 단계 2 당시 구성에서 YAML 편집 없이 두 독립 풀의 Compose 1→2→1과 2+2 분산을 확인했다. [당시 풀 검증 이력](history/2026-11-13-pool-validation.md)은 현재 d1 복제 검증이 아니다.
- Compose 자체에는 자동 롤링 업데이트가 없다. 요청 제외 → 드레이닝 → 교체 → 준비 확인 절차를 별도로 구성해야 한다.
- 당시 준비 전 제외·진행 요청 완료 후 축소·재생성·IP 재탐색을 확인했다. 전체 풀 재생성은 계획 중단이며 운영 무중단 배포나 현재 제품 구성의 구현 완료가 아니다.

## 제약과 확인 필요 사항

- Caddy 자동 탐색·제거, 준비 반영, 요청 분산은 당시 Docker Desktop 구성에서 확인했다. Rust+d1+Caddy 및 운영 네트워크·ARM64에서 별도 검증해야 한다.
- d1 Q4 두 개의 동시 로딩·실제 분산·최대 메모리·처리량·CPU 효율·Oracle 성능은 미검증이다. 과거 엔진 두 복제본 성공을 이식하지 않는다.
- Gateway 다중 프로세스의 로컬 SQLite 제한·폐기·쓰기 잠금 실패 처리는 단계 3에서 확인했다. 운영 Migration 단일 실행 보장, 백업 복구, 신·구 스키마 공존 및 실제 부하 성능은 후속 검증이 필요하다.
- Docker Desktop 단계 2 실험은 운영 구성 검증이 아니다. 환경 경계는 [로컬 개발 환경](local-development.md), 모델 품질·지연은 [추론 Wiki](inference.md)에 둔다.

## 근거와 관련 결정

- [README](../../README.md): SQLite·Docker Compose·GitHub Actions 사용 방향.
- [인계된 설계 합의](history/2026-11-12-initial-design.md): 최초 구성의 역사적 근거.
- [d1 Q4 채택](history/2026-11-13-d1-q4-adoption.md): 모델·정리·단계 4 진행 승인과 운영 승인 분리. CI·Migration·운영 배포 검증은 아직 없다.
