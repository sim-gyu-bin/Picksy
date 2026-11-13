# DB와 배포

[Wiki](wiki.md)

## 현재 지식

**단계 4·4A~4C 구현·로컬 검증 완료 · 운영 배포 미구현.** 실제 추론, 관리자 데이터·Google 인증·관리 HTTP, React 화면의 근거는 단계별로 구분한다. 4C는 호스트 Rust의 실제 Google 화면 흐름과 amd64 서버 컨테이너의 정적 제공·API 경계를 확인했다. 컨테이너의 실제 Google 로그인·제품 풀·CI 원격 실행·ARM64·운영 백업/복구·배포는 미검증이다.

### 관리자 화면과 서버 이미지

- [정적 제공](../../src/static_ui.rs)의 `router`는 알려진 화면·자산만 제공하고 `openat`·`O_NOFOLLOW`로 symlink 경계와 일반 파일 여부를 검사한다. API 오류를 HTML로 대체하지 않는다.
- [Dockerfile](../../deploy/Dockerfile)은 Node 22.21.1의 정적 빌드와 Rust 1.95.0의 서버 빌드를 분리한다. 최종 이미지는 비root UID/GID 10001이며 실제 실행에서 Node/npm 부재와 정적 자산 3개를 확인했다. 이미지 빌드 입력은 [.dockerignore](../../.dockerignore)의 허용 목록으로 제한한다.
- DB는 named volume의 `/var/lib/picksy/data/gateway.sqlite3`에 두며 Migration을 명시적으로 실행한다. 이미지 기본 명령은 도움말이다. 운영 자동 Migration·공개 포트·Caddy 배치를 이 이미지에 내장하지 않았다.
- 4C에서는 새 격리 호스트 DB와 별도 named volume만 사용했다. 실제 브라우저 키 발급·복사·확인 폐기·로그아웃, UTC 필터·NULL/0·모바일·watch를 확인했다. 그래프 이력 3행은 합성 데이터이며 d1 성능 근거가 아니다. 명령·상세 결과·남은 검증은 [4C 결과](../plans/implementation-plan.md#단계-4c-결과), 실행 방법은 [로컬 개발](local-development.md)을 따른다.

### DB와 Migration

- SQLite + SQLx를 사용하고 운영 DB는 Docker named volume에 영구 보관한다.
- 로컬 DB와 운영 DB를 분리한다. 로컬 DB 파일을 운영에 복사하지 않는다.
- Gateway를 두 개 실행할 경우 같은 VM의 DB 볼륨을 공유한다.
- AI가 Migration을 작성하고 검사한다. 승인된 배포에서 백업 → Migration 단일 실행 → 성공 시 서비스 배포 순서를 따른다.
- 신·구 코드가 공존할 수 있는 스키마 변경을 우선한다. 이미지 롤백은 DB 롤백이 아니다.
- v2 스키마는 구 바이너리의 엄격한 Migration 검사에서 거절된다. 이를 우회하지 않았으며 신·구 바이너리의 무중단 스키마 호환성은 미검증이다. 새 격리 DB의 v1→v2 검증은 완료했지만 기존 로컬·운영 DB는 변경하지 않았다. 운영 Migration과 이미지 롤백·DB 복구는 별도 승인·검증 대상이다.

### 단계 4까지 구현·검증한 키·사용량·제한

- [초기 Migration](../../migrations/0001_gateway.sql)은 키 해시·키별 현재 분 카운터·요청 메타데이터를 저장한다. [DB 코드](../../src/db.rs)의 `migrate`만 생성·Migration하며 `Store::open`은 기존 스키마를 검사한다. 서버 기동·키 명령이 자동 Migration하지 않는다.
- SQLite는 WAL, `synchronous=FULL`, 외래 키 활성화, 쓰기/풀 획득 대기 5초, 프로세스당 최대 연결 4개다. 운영의 승인된 백업과 Migration 단일 실행 절차를 대체하지 않는다.
- 로컬 CLI는 키 발급·목록·폐기·사용량을 제공한다. 키는 `pk_` + OS 난수 256비트의 16진수이며 원문을 한 번만 출력한다. 저장 검증값은 SHA-256이고 목록에 해시도 노출하지 않는다. [정책 선택과 이유](history/2026-11-13-gateway-key-policy.md), [실행 명령](local-development.md).
- `Store::admit`은 `BEGIN IMMEDIATE` 안에서 유효 키·폐기 여부·고정 UTC 분 한도를 검사하고 카운터와 사용량 접수를 함께 커밋한다. 기본 한도는 키별 서비스 전체 60회/분이다. 각 Gateway에 별도 DB를 연결하면 공유 제한이 아니므로 같은 로컬 파일/볼륨을 사용해야 한다.
- 인증 실패 401은 DB 사용량에서 제외한다. 인증 후 한도 거절 429는 사용량에 포함하되 카운터를 추가 차감하지 않는다. 허용 요청은 pending으로 접수하고 응답 전에 완료 상태·소요 시간을 기록한다. 완료 쓰기 실패는 503이며 pending을 성공으로 집계하지 않는다.
- 집계는 `total = pending + succeeded + rejected + failed + other_completed`다. `rejected`는 기록된 4xx이며, 접수 전 DB 장애와 인증 실패는 포함하지 않는다. HTTP 전달 성공과 DB의 완료 상태는 동일하지 않다. 자동 삭제·자동 보존 만료는 없으며 사용량 데이터는 누적된다.
- [HTTP 미들웨어](../../src/gateway.rs)는 매 요청 DB 검사로 폐기를 반영하고 원문 Authorization을 후속 핸들러에 넘기지 않는다. 요청 ID·상태·소요 시간만 로그에 남긴다. 단계 4에서 추론 경로·취소·대기열 제한을 연결했다.

### 단계 4A 스키마 확장

공통 데이터 기반과 새 Migration·트랜잭션·CLI·실제 추론 토큰 영속성을 구현·로컬 검증했다. 구체적인 실행 근거는 아래 4A 절에 두며 단계 3·4 당시 수치도 보존한다.

| 대상 | 새 Migration `0002` |
|---|---|
| `api_keys` | nullable `name`. 기존 키·검증 해시·행을 유지한다. |
| `request_usage` | nullable `request_kind`, `input_tokens`, `output_tokens`, 종류 CHECK·조회 인덱스. 두 토큰은 모두 NULL이거나 모두 비음수이며 실제 inference 성공에서만 기록한다. |
| `admin_sessions` | UUID `id`, unique 32바이트 `token_hash`, 32바이트 `csrf_token_hash`, `google_sub`, 32바이트 `admin_email_hash`, `created_at`, `expires_at`, `revoked_at`. |
| `admin_login_states` | 32바이트 `state_hash` 기본 키, `nonce_hash`, `browser_binding_hash`, `created_at`, `expires_at`, `consumed_at`. |
| `admin_events` | UUID `id`, `action`(`key_created`/`key_revoked`), `key_id`, `actor_kind`, 선택적 `actor_session_id`, `created_at`. |

- `0001`은 수정하지 않는다. 기존 이름·요청 종류·토큰은 NULL로 유지하며 과거 결과를 추정 backfill하지 않는다. 키·사용량·감사 이력을 삭제하지 않는다.
- 4A 당시 세션·일회성 로그인 상태는 길이·시각 순서·필수값 CHECK와 만료 조회 인덱스를 가진 스키마만 준비했다. 당시 실제 Google 로그인·계정 검증·세션 토큰 발급·관리 HTTP는 미구현이었다. 4A 검증에서는 두 테이블이 비어 있으며 실제 계정 값이나 로그인 토큰을 생성·저장하지 않았다. 후속 4B의 런타임·추가 Migration은 아래 별도 절을 따른다.
- 감사의 키 참조는 FK로 연결한다. 행위자는 `cli`/`admin`과 관리 작업의 세션 UUID만 기록하며 이름·이메일·키 원문·인증 토큰을 감사에 복제하지 않는다. `actor_session_id`는 향후 세션 삭제와 독립된 역사 식별자이며 세션 FK로 묶지 않는다.
- 생성·폐기와 감사 기록은 같은 `BEGIN IMMEDIATE` 트랜잭션이다. 삽입 실패 시 모두 롤백하고 중복 폐기는 추가 이벤트 없이 기존 상태를 유지한다. 공통 이름·표시명·집계 계약은 [API 문서](../api-contract.md#단계-4a-키사용량-확장-계약)를 따른다.
- 완료 상태·소요 시간·검증된 실제 추론 토큰은 원자적으로 기록한다. 실패·취소·quota 거절의 토큰은 NULL이다. 집계는 종류별 수집/미수집을 구분하며 실제 0과 NULL을 합치지 않는다. 전체·UTC 일별·종류별 결과는 동일 읽기 snapshot에서 산출한다.
- `validate_schema`와 readiness의 필수 스키마 확인은 새 열·테이블을 포함한다. 서버 시작은 기존 DB를 자동 Migration하지 않는다. 4A는 새 격리 v1 DB를 명시적으로 v2로 올려 검증했으며 기존 로컬·운영 DB는 변경하지 않았다.

### 단계 4A 실행 근거

- `cargo fmt --all && bash scripts/check.sh && cargo build --locked --bin picksy` 성공. CLI 2개·admin_data 12개·gateway 11개·inference 20개, 총 45개 테스트를 통과했다. 아래 실제 DB·CLI·d1 실행과 격리 회귀 테스트를 구분한다.
- `.local/picksy-dev/stage4a-c42595bb2a/legacy.sqlite3`를 v1 바이너리로 새로 생성한 뒤 v2 Migration을 실행했다. 기존 Migration checksum·키 해시·기존 요청 2개 행을 보존했고 name·request_kind·tokens는 NULL이었다. 기존 v1 키는 새 서버에서 auth 204로 동작했다. 기존 요청을 추론으로 추정하지 않았다.
- CLI에서 이름 trim, 이름 미입력 시 UUID 표시, UUID 식별자·256비트 비밀키 자동 생성, 잘못된 기간 거부를 확인했다. UTC `[1704067200, 1704153600)` 조회는 1월 1일 자료 1건만 반환했다.
- 별도 격리 복제 DB 2개에서 감사 삽입을 강제로 실패시켰다. 키 생성·폐기 모두 실패하고 키 변경과 감사가 함께 롤백됐다. 중복 폐기는 최초 폐기 시각과 이벤트 1건을 유지했다. 이 fault DB는 의도적인 실패 trigger를 남긴 검증 자료이며 서비스로 실행하지 않는다.
- 실제 고정 d1 Q4의 추론 1회는 POST 200, input 70·output 0으로 응답과 DB가 정확히 일치했다. 잘못된 입력 400의 토큰은 NULL이었다. 명명 키의 inference 집계는 2건(성공 1·거절 1), `token_recorded=1`, `token_unknown=1`, input 70·output 0이었다.
- 전체 요청 6건은 `auth_check` 1·`inference` 2·`models` 1·기존 `unknown` 2였다. 모델 조회에 `kind=inference` query를 넣어도 models로 기록됐다. 폐기 후 추론은 401이고 과거 usage는 유지됐다.
- 주 검증 DB의 모든 키를 폐기했다. 최종 감사는 `key_created` 2건·`key_revoked` 3건, `admin_sessions`·`admin_login_states`는 각각 0행이다. 로그인 기능의 성공 근거가 아니다.
- 키·본문 없는 메타데이터 보고서는 `.local/picksy-dev/stage4a-c42595bb2a/report.json`이다. 새 Picksy Server SIGTERM exit 0, d1 컨테이너 stop exit 0·OOM false, 로그·DB/WAL의 원문 키 미노출을 확인했다. 기존 로컬·운영 DB와 모델 파일은 변경하지 않았으며 파일·컨테이너 자동 삭제도 하지 않았다.
- 구 바이너리의 v2 DB 접근 제한을 유지한다. 운영 Migration·롤링 호환·ARM64·Google 인증·관리 HTTP·화면은 이번 결과에 포함하지 않는다.

### 단계 4B 관리자 세션·공유 제한

구현·로컬 검증한 관리 경계는 [4B HTTP 계약](../api-contract.md#단계-4b-관리자-인증관리-http-계약)을 따른다. 4A의 빈 세션 테이블·검증 결과는 당시 상태이며 4B 로그인 성공 근거로 재사용하지 않는다.

- [0003_admin_auth.sql](../../migrations/0003_admin_auth.sql)은 `admin_sessions.config_hash`를 추가한다. nullable 32바이트 설정 fingerprint이며 기존 세션에는 증거를 추정 backfill하지 않고 NULL을 유지한다. [관리 인증](../../src/admin.rs)의 `authenticate`는 현재 fingerprint와 일치하는 세션만 허용하므로 NULL 세션도 거부한다.
- `admin_rate_windows`는 `login`, `callback`, `admin`의 고정 버킷과 `window_start`, `admitted`를 저장한다. 클라이언트 IP·전달 헤더를 신뢰하는 제한이 아니라 같은 SQLite를 쓰는 복제본 전체의 공유 제한이다. 추론 API의 키별 quota와 분리한다.
- 기존 `0001`·`0002`의 키·사용량·감사 이력은 보존한다. 새 Migration 작성은 적용 성공을 뜻하지 않으며 서버 시작·ready 검사를 우회하거나 기존 로컬·운영 DB를 자동 Migration하지 않는다.
- 다중 서버는 동일 DB와 동일 관리자/OAuth 설정을 사용해야 한다. 격리 회귀 테스트와 아래 실제 브라우저·HTTP 실행 근거를 구분하며 로컬 검증을 운영 복제·부하 검증으로 확대하지 않는다.
- 운영은 별도 승인 후 백업 → Migration 단일 실행 → 성공 시 배포 순서를 유지한다. v3 스키마의 구 바이너리 호환·이미지 롤백·DB 복구·ARM64·E3 운영 배포는 이 문서 추가로 보장하지 않는다.

### 단계 4B 실행 근거

- 로컬에서 `cargo fmt`, Clippy의 `-D warnings`, `cargo test --locked` 66개(기존 45·관리 HTTP 15·OIDC/세션 단위 6), `cargo build --locked --bin picksy`가 각각 통과했다.
- 새 격리 DB는 `.local/picksy-dev/stage4b-1f9a4b7710/admin.sqlite3`, 메타데이터 보고서는 같은 디렉터리의 `report.json`이다. 기존 로컬·운영 DB는 변경하지 않았다.
- 실제 브라우저의 Google 로그인 후 session 200과 `HttpOnly`, `SameSite=Lax`, `Cache-Control: no-store`, `Referrer-Policy: no-referrer`를 확인했다. 이름 있는 키 생성은 201, 발급 키의 Bearer auth는 204였다. Bearer로 관리 API를 호출하거나 관리자 쿠키로 추론 인증을 대신하면 각각 401이었다.
- CSRF 누락·잘못된 Origin은 403, 키 목록에는 원문 비밀키가 없었다. 관리 usage의 `total=1`, 입력 토큰 NULL을 확인해 관리 호출이 추론 통계에 포함되지 않음을 확인했다. 잘못된 기간은 400이었다.
- 키 DELETE 반복 호출은 각각 204이고 생성 감사 1건·폐기 감사 1건을 유지했다. 폐기 키의 Bearer 요청은 401, 로그아웃은 204, 폐기 세션 재사용은 401이었다.
- 검증 프로세스의 허용 계정 설정만 합성 값으로 바꿔 재기동하고 같은 Google 계정으로 실제 로그인했을 때 callback 403 `forbidden`과 새 세션 미발급을 확인했다. 비공개 `.env`는 변경하지 않았다. 최종 DB는 세션 1건·폐기 1건·소비된 로그인 상태 2건이었다.
- DB·로그에 이메일·client secret·API 키·세션·CSRF 원문이 없음을 확인했다. 시험 키와 세션을 폐기했고 검증 서버는 SIGTERM exit 0으로 종료했다. 실제 값이나 민감 본문은 보고서에 기록하지 않는다.
- 실제 inference upstream은 기동하지 않은 관리 기능 독립 검증이며 live 200·ready 503이었다. 단계 4의 실제 추론 검증과 구분한다. React 4C·Caddy 제품 풀·Oracle ARM64·운영 배포 검증은 아니다. 공개 login의 전역 30회/분 한도를 익명 요청이 소진할 수 있는 잔여 DoS 위험은 남아 있다.





### 단계 3 실행 근거

- Intel Mac `x86_64-apple-darwin`, Rust 1.95.0에서 `bash scripts/check.sh`의 포맷 확인·Clippy 경고 거부·통합 테스트 9개와 `cargo build --locked --bin picksy`를 통과했다. [CI](../../.github/workflows/check.yml)는 같은 검사 진입점을 사용하지만 원격 Actions 실행은 아직 확인하지 않았다.
- 실제 CLI로 Migration·키 생성·목록·폐기·사용량을 실행하고, 두 독립 서버 프로세스에서 누락/잘못된/중복 인증 401, 유효 키 204, 폐기 및 서버 재시작 뒤 401을 확인했다.
- 한도 5인 키로 두 프로세스에 동시 12건을 보내 총 5건 204·7건 429를 관측했다. SQLite 쓰기 잠금을 잡으면 503, 해제 후 204였고 한도를 우회하지 않았다.
- 실제 집계는 성공 8·거절 7·pending 0·전체 15였다. DB·WAL·로그의 원문 키 미노출을 확인했다. 완료 기록 실패·접수 롤백·429 기록 실패·DB 오류·명시적 Migration 경계는 격리 통합 테스트로 확인했다.
- 검증 서버를 정상 종료했고 데이터는 `.local/picksy-dev/stage3-667b8a9c95/` 및 테스트별 고유 폴더에 보존했다. 운영 DB·Rust+d1+Caddy 통합·OMP·ARM64는 이 검증에 포함하지 않았다.

### 단계 4 사용량·종료 확인

- 실제 설치 OMP→Rust Gateway→d1의 정상·입력 오류·과부하·timeout·상류 장애를 기록했다. 정상 설정의 최종 상태별 건수는 200 6건·400 3건·499 1건·503 11건, 별도 timeout 설정은 504 1건, 연결 불가 설정은 502 1건이다. 최종 pending은 0이었다.
- 실제 50ms AbortSignal 취소 후 499 기록·permit 반환·후속 정상 추론을 확인했다. DB 실패·런타임 강제 종료에는 pending이 남을 수 있으며 엔진 CPU 계산의 즉시 중단 시간은 계측하지 않았다.
- Gateway 3개 SIGTERM exit 0·d1 컨테이너 stop exit 0/OOM false, 시험 키 3개 폐기, Gateway·d1 로그와 새 DB/WAL의 원문 키 미노출을 확인했다. 원문 요청·응답·키를 저장하지 않는 메타데이터 보고서는 `.local/picksy-dev/stage4-ce4ada0a10/report.json`이다. 파일·컨테이너 자동 삭제는 하지 않았다.
- 실행 환경·27개 테스트와 실제 추론 검증의 구분은 [단계 4 결과](../plans/implementation-plan.md#단계-4-결과)를 따른다. 운영 DB·백업·복구·배포 검증은 아니다.



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
