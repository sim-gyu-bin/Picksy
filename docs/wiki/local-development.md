# 로컬 개발 환경

[Wiki](wiki.md) · [DB와 배포](operations.md) · [작업계획서](../plans/implementation-plan.md)

## 실행 전제

Intel Mac의 Rust 호스트 타깃은 `x86_64-apple-darwin`, Docker 엔진은 `linux/amd64`다. Oracle 운영 타깃 `linux/arm64`와 구분한다. 확인한 도구 버전과 명령 결과는 작업계획서 단계 0에 기록한다.

Docker CLI와 Compose가 설치되어 있어도 Desktop 엔진이 실행 중이어야 한다. 엔진 소켓 연결이 실패하면 Docker Desktop 실행 상태부터 확인한다. 정상 연결 확인에는 `docker version`, `docker info`, `docker compose ls --format json`을 사용한다. 마지막 명령의 빈 배열은 실행 중인 Compose 프로젝트가 없다는 뜻이며 컨테이너 실행 검증은 아니다.

## 로컬 실험 경계

- Compose 프로젝트 이름은 `picksy-dev`로 한다. 운영은 별도 프로젝트와 별도 볼륨을 사용한다.
- 로컬 Caddy 진입점은 `127.0.0.1:18080`을 예약한다. 직접 Gateway를 실행하는 개발 모드에는 `127.0.0.1:18081`을 예약한다. 실제 실행 전 포트 충돌을 확인한다.
- 목표 Compose 풀의 Gateway·d1 개별 인스턴스에는 고정 호스트 포트를 할당하지 않는다. 내부 Caddy 리스너와 d1은 호스트에 공개하지 않는다. Rust+d1+Caddy 재통합은 미검증이며 컨테이너 내부 서비스 포트는 실제 서버 계약을 확인한 뒤 정한다.
- 로컬 직접 실행의 DB·모델·실험 결과는 저장소의 `.local/picksy-dev/` 아래에 둔다. 실험 모델·결과는 이미 있으며 운영 DB와 혼용하지 않는다.
- Compose DB는 후속 구현에서 프로젝트 범위 named volume을 사용한다. 개발 볼륨은 `picksy-dev` 범위에 속하도록 하고 운영 볼륨을 external volume이나 직접 경로로 연결하지 않는다. 서비스명·볼륨 키·컨테이너 DB 경로는 아직 미정이다.
- 로컬 데이터는 운영으로 복사하지 않는다. 실제 키나 운영 데이터 대신 합성 입력을 사용한다. 볼륨 삭제·초기화·`docker compose down -v`는 별도 승인 없이 실행하지 않는다.

## 설정 예시와 상태

[.env.example](../../.env.example)은 비밀값이 없는 초기 로컬 설정 예시다. `COMPOSE_PROJECT_NAME`은 Compose 변수이고, `PICKSY_DEV_*`는 후속 구성용 예약 변수다. 실제 단계 2 설정이 이를 읽는지는 해당 구현과 검증 결과로 판단하며 예시만으로 적용을 보장하지 않는다.

[.gitignore](../../.gitignore)는 실제 `.env` 계열, `.local/`, 루트 `target/`을 제외하고 `.env.example`은 보존한다. ignore는 비밀값 유출 방지의 보조 장치일 뿐, 이미 추적된 파일이나 로그를 보호하지 않는다.

단계 0에서는 컨테이너·볼륨 생성, 모델 다운로드, Rust 앱 빌드, ARM64 검증을 수행하지 않았다. 실험용 서버 실행은 후속 단계에서 필요한 범위만 진행한다.

## Google 로그인 개발 설정

- E1의 비공개 로그인 설정 준비는 완료했다. 실제 계정·프로젝트 식별자·인증 JSON은 문서에 기록하지 않으며 운영 공개·도메인 설정은 E3 범위다.
- 등록한 JavaScript origin은 `http://127.0.0.1:18081`, 리디렉션 URI는 `http://127.0.0.1:18081/auth/google/callback`이다. 서버측 OIDC 코드 교환을 사용하며 브라우저에 client secret을 전달하지 않는다. `localhost`나 Caddy의 18080 포트는 현재 등록값과 다르므로 임의로 혼용하지 않는다.
- 비공개 `.env`에는 `PICKSY_ADMIN_GOOGLE_EMAIL`, `PICKSY_GOOGLE_CLIENT_ID`, `PICKSY_GOOGLE_CLIENT_SECRET`, `PICKSY_GOOGLE_REDIRECT_URI`를 보관한다. 공개 예시는 빈 값만 둔다. 다운로드한 인증 JSON도 비밀 파일로 취급하며 Git·로그에 추가하지 않는다.
- `.env` 자동 로딩은 없다. 실행 주체가 위 네 변수를 서버 프로세스 환경에 명시적으로 주입해야 하며, 파일을 준비한 사실만으로 서버 설정 적용을 보장하지 않는다. 비밀값을 명령행 인자·셸 추적 출력·프론트 번들에 넣지 않는다. Google 테스트 사용자 등록 자체는 Picksy 관리자 인증을 대신하지 않는다.
- 공식 근거는 [Google OpenID Connect](https://developers.google.com/identity/openid-connect/openid-connect)다. 4B에서 실제 인증·관리 HTTP를, 4C에서 실제 React 로그인·키 관리 화면을 로컬 검증했다. 상세 결과는 [DB와 배포](operations.md)에 두며 4D 종합 검증·운영 E3·ARM64 검증을 구분한다.
- 4B의 `AdminConfig::from_env`는 위 네 환경변수를 읽는다. 모두 미설정이면 관리 경로는 503으로 비활성화되고, 빈 값·일부만 설정·잘못된 callback은 기동 오류다. `.env.example`의 빈 값은 실행 가능한 인증 설정이 아니다.
- HTTP 개발 callback은 literal loopback IP만 허용하며 서버 bind도 loopback이고 callback 포트와 같아야 한다. 운영은 HTTPS callback과 Secure 쿠키를 사용한다. 설정 변경은 재시작 후 기존 세션·진행 중 로그인을 차단하며 복제본에는 동일 설정을 주입한다.
- 로그인 시작은 `GET /auth/google/login`이고 callback 성공은 고정 `/console`로 303 이동한다. 4C에서 과거 성공 세션 JSON을 화면 이동으로 대체했고 오류 상태·JSON은 유지한다. 요청 형식은 [관리 HTTP 계약](../api-contract.md#단계-4b-관리자-인증관리-http-계약), 구현은 [관리 인증](../../src/admin.rs), 스키마는 [0003](../../migrations/0003_admin_auth.sql)을 따른다. 실제 검증은 새 격리 DB·명시적 환경 주입으로 수행했으며 기존 `.env`·로컬/운영 DB는 변경하지 않았다.

## 관리자 화면 개발

4C 구현·로컬 브라우저·amd64 이미지 검증을 완료했다. `admin/`의 React·TypeScript·shadcn/ui·Tailwind 화면은 esbuild·Tailwind CLI로 빌드하며 Vite·Next.js·별도 프론트 개발 서버를 사용하지 않는다. Node 22.21.1·npm 10.9.4를 확인했고 정확한 의존성은 `admin/package-lock.json`으로 고정한다. 평소 화면 개발은 Rust 서버와 watch를 실행한 뒤 새로고침하며 Docker 재빌드가 필요 없다. HMR은 제공하지 않는다.

```sh
npm --prefix admin ci
npm --prefix admin run typecheck
npm --prefix admin run build
# 변경 시 자산만 다시 빌드한다. HTTP 서버나 HMR 서버를 띄우지 않는다.
npm --prefix admin run watch
```

- Picksy Server의 `serve --admin-assets admin/dist`가 화면을 제공한다. `/console`은 사용량, `/console/keys`는 키 관리 화면이며 자산 경로는 `/console/assets/app.js`·`/console/assets/app.css`다. 개발은 기존 서버 주소에서 브라우저를 새로고침한다.
- 로그인 후 콜백은 고정된 `/console`로 이동한다. API는 기존 `/admin/*` 경로와 세션·CSRF 계약을 사용한다. 화면 shell은 공개되지만 관리 데이터·비밀 환경값은 포함하지 않으며 관리자 API 인증을 대신하지 않는다.
- 원문 API 키는 발급 직후 화면 메모리에서만 표시·복사한다. 대화상자 닫기·페이지 이동·로그아웃 후 다시 조회할 수 없고 브라우저 저장소·URL에 기록하지 않는다. 폐기는 이력 삭제가 아닌 신규 요청 차단이다.
- 기간 필터는 UTC 기준이며 종료일 포함 UI는 서버의 배타적 `until` 경계로 변환한다. NULL 토큰은 미수집으로 표시하며 실제 0과 구별한다. 실제 조회 오류를 빈 데이터·0으로 숨기지 않는다.
- 자산은 알려진 파일만 제공하고 API 오류에 SPA fallback을 적용하지 않는다. 응답은 `no-store`로 개발 갱신과 민감 화면 캐시 경계를 단순하게 유지한다. CSP의 스크립트는 동일 출처만 허용한다. Radix 대화상자·스크롤 잠금의 동적 스타일을 위해 스타일에만 `unsafe-inline`을 허용한다.
- `admin/dist`·`admin/node_modules`는 생성물이며 Git에 넣지 않는다. 빌드에 `.env`·서버 비밀 환경변수를 주입하지 않는다. Node 실행 파일과 개발 의존성은 최종 서버 이미지에 포함하지 않는다.

## 서버 이미지의 로컬 실행

4C 서버 이미지는 프론트 빌드 단계와 Rust 빌드 단계를 분리한다. 최종 이미지에는 Rust 실행 파일·정적 파일·TLS 인증서만 두며 Node 서버를 실행하지 않는다. 이미지 기본 명령은 도움말이고 Migration이나 서버 기동을 자동 수행하지 않는다.

```sh
docker build -f deploy/Dockerfile -t picksy-server:local .
# 새 개발 전용 볼륨에서 명시적으로 Migration한다. 기존/운영 볼륨에 재사용하지 않는다.
docker volume create picksy-admin-dev
docker run --name picksy-admin-migrate \
  --mount type=volume,source=picksy-admin-dev,target=/var/lib/picksy \
  picksy-server:local migrate
```

- 이미지의 DB 경로는 `/var/lib/picksy/data/gateway.sqlite3`이고 정적 파일은 `/app/admin`이다. 서버 실행은 `serve --bind 0.0.0.0:18081 --admin-assets /app/admin --inference-url <내부-origin>`을 명시하고, 내부 인증 값은 비공개 실행 환경으로 전달한다. 예시 컨테이너·볼륨은 자동 삭제하지 않는다.
- 호스트 개발 공개 포트는 `127.0.0.1`에만 게시한다. 제품 구성은 단계 5에서 Caddy만 공개하며 Picksy Server·d1은 직접 노출하지 않는다. 이 이미지 자체는 Compose 제품 풀이나 운영 배포 설정이 아니다.
- Google HTTP 개발 callback의 loopback bind 검사를 Docker 시험 편의로 해제하지 않는다. 호스트 Rust에서는 기존 E1 설정으로 실제 로그인을 검증하고, 브리지 네트워크의 컨테이너에서는 정적 제공·관리 API의 인증/설정 경계를 검증한다. 실제 컨테이너 Google 로그인은 HTTPS·등록 callback·프록시 구성을 갖춘 별도 검증이 필요하다.


## 현재 추론과 과거 실험

현재 채택은 [d1 Q4](history/2026-11-13-d1-q4-adoption.md)다. 고정 모델·이미지·실측은 [추론 Wiki](inference.md), 실행기는 [experiments/d1](../../experiments/d1/README.md), 공통 입력은 [short-cases.json](../../experiments/fixtures/short-cases.json)에 둔다. `.local/picksy-dev/d1-q4/` 모델·보고서는 보존한다.

과거 모델·실험 원본·전용 Docker 자원의 정리 경계는 [채택 기록](history/2026-11-13-d1-q4-adoption.md#승인된-정리-범위와-보존)을 따른다. 과거 수치·실패·해시·출처는 [비교 History](history/2026-11-13-inference-comparison.md), 풀 구성은 [풀 History](history/2026-11-13-pool-validation.md)에 남긴다. Rust 코드·DB·History·공용 이미지는 보존한다. 단계 4 로컬 검증은 완료했으며 단계 5는 미착수이고 운영 승인이 아니다.

## Picksy Server 실행

루트의 단일 `picksy` crate를 사용한다. 서버 전체 명칭은 Picksy Server이며 Gateway는 내부 인증·추론 중계 역할이다. 단계 4A 공통 키·사용량·Migration의 로컬 검증도 완료했다. 아래 DB 경로는 개발용 예시이며 기존 DB 업그레이드의 자동 승인이 아니다. 4A 검증은 새 격리 DB만 사용했고 기존 로컬·운영 DB는 변경하지 않았다. `.env`·`PICKSY_DEV_*`·`RUST_LOG` 자동 로딩을 가정하지 않는다. 내부 추론 키는 전용 환경변수로 받는다.

```sh
cargo build --locked --bin picksy
# 로컬 DB만 생성·Migration한다. 운영에서는 승인된 백업·단일 실행 절차가 별도로 필요하다.
./target/debug/picksy --db .local/picksy-dev/gateway.sqlite3 migrate
./target/debug/picksy --db .local/picksy-dev/gateway.sqlite3 key create --requests-per-minute 60
./target/debug/picksy --db .local/picksy-dev/gateway.sqlite3 key list
```

`key create`는 `id`·선택적 `name`·`display_name`과 원문 `api_key`를 반환하며 원문 키는 생성 시 한 번만 제공한다. 생성과 감사 기록은 같은 트랜잭션이다. `key revoke <id>`도 폐기와 감사를 함께 기록하고, 이미 폐기된 키는 최초 시각을 유지하며 추가 이벤트를 만들지 않는다. 출력은 로그·문서·저장소에 붙이지 않는다. 원문 키는 복구할 수 없으므로 새 키 발급과 기존 키 폐기를 사용한다. 이름·기간/종류별 usage 계약은 아래를 따른다.

`serve`·키 관리·사용량 조회는 기존 DB와 Migration 버전·체크섬·필수 열을 검증하며, DB가 없거나 스키마가 다르면 자동 생성·수정하지 않고 실패한다. 생존 `/health/live`, DB+upstream 준비 `/health/ready`, 보호 API `GET /v1/auth/check`와 단계 4 추론 경로를 제공한다. 실제 OMP 종단·오류·취소의 검증 범위는 [HTTP 계약](../api-contract.md)과 [단계 4 결과](../plans/implementation-plan.md#단계-4-결과)를 따른다.

검사 진입점은 로컬과 [CI](../../.github/workflows/check.yml) 모두 `bash scripts/check.sh`다. 먼저 `npm --prefix admin ci`로 잠긴 의존성을 준비하며 공통 검사는 TypeScript·프론트 빌드·프론트 회귀·Rust 포맷·Clippy·Rust 테스트를 수행한다. 테스트는 합성 키와 고유한 격리 DB를 사용하고 운영·유료 API를 연결하지 않는다. 자동 삭제하지 않으므로 반복 실행하면 로컬 테스트 데이터가 누적된다.

소스는 [CLI](../../src/main.rs), [DB](../../src/db.rs), [HTTP](../../src/gateway.rs), [초기 Migration](../../migrations/0001_gateway.sql), [경계 테스트](../../tests/gateway_boundaries.rs)에 둔다. 초기 Migration·실제 실행 완료 여부는 [작업계획서](../plans/implementation-plan.md)와 [DB와 배포](operations.md)를 따른다.

## 단계 4A 키·이름·사용량 CLI

CLI·공통 데이터 기반을 구현하고 새 격리 DB에서 검증했다. [실행 결과](operations.md#단계-4a-실행-근거)는 기존 v1 자료 보존·감사 원자성·필터·실제 토큰 저장을 포함한다. 기존 로컬·운영 DB 변경은 이 결과로 자동 승인되지 않는다. 후속 Google 로그인·관리 HTTP는 [4B 계약](../api-contract.md#단계-4b-관리자-인증관리-http-계약)을 따른다.

- `key create --name <이름>`은 선택 사항이며 기존 `--requests-per-minute`와 함께 사용한다. 이름은 trim 후 빈 값이면 NULL, 최대 128 Unicode scalar 문자, 제어문자 금지, 중복 허용이다. 이름을 생략하면 `display_name`은 자동 생성한 UUID다. `name`·`display_name`은 생성·목록 결과에 추가하며 비밀키와 UUID는 계속 서버가 생성한다.
- CLI 생성·폐기는 행위자 `cli`로 감사 기록을 같은 트랜잭션에 남긴다. 감사 실패 시 변경도 롤백한다. 중복 폐기는 새 이벤트 없이 기존 폐기 시각을 유지한다. 4A 당시에는 관리자 HTTP 호출·Google 로그인을 추가하지 않았다.
- `usage [--key-id <UUID>] [--from <UTC Unix 초>] [--until <UTC Unix 초>] [--kind <종류>]`로 조회한다. 기간은 `[from, until)`이며 음수와 `from >= until`인 기간을 거절한다. 한쪽 경계 생략은 해당 방향 제한 없음이다.
- `--kind`는 `all`(기본), `inference`, `models`, `auth-check`, `admin`, `unknown`이다. CLI의 `auth-check`와 데이터·JSON enum의 `auth_check`를 구별한다. `admin` 분류값은 유지하지만 관리 HTTP를 추론 통계에 넣지 않는다.
- CLI JSON은 `key_id`·`usage` 요약과 `from`, `until`, `kind`, `by_day`, `by_kind`를 제공한다. 공통 Rust `UsageReport`의 요약 접근은 `.summary`이고 CLI에서는 그 요약이 `usage`에 들어간다. 일별·종류별 각 항목도 `summary`를 갖는다. 기존 공통 API의 호출자는 호환 shim 없이 새 계약으로 이행했다.
- 토큰 합계는 수집값이 없으면 NULL, 실제 0을 수집했으면 0이다. `token_recorded`·`token_unknown`은 분류된 inference만 대상으로 하며 과거 NULL 종류는 `unknown`으로 별도 집계한다. 취소·오류·quota 거절 토큰을 0으로 만들지 않는다. 상세 snapshot·overflow·키 이력 계약은 [API 문서](../api-contract.md#단계-4a-키사용량-확장-계약)를 따른다.

4A의 `0002`는 기존 이름·종류·토큰을 추정하지 않으며 당시 세션·로그인 일회성 상태는 스키마만 준비했다. 새 필수 열·테이블의 시작/ready 검사와 엄격한 Migration 검사를 유지한다. 구 바이너리는 v2 DB를 새로 열 수 없으므로 이미지 교체만으로 DB 복귀가 가능하다거나 무중단 호환이라고 가정하지 않는다. 후속 4B를 포함한 [스키마·Migration 경계](operations.md#단계-4a-스키마-확장)를 따른다.

검증에는 합성 값만 사용했고 이름·비밀키·개인정보를 로그·감사에 복제하지 않았다. 실제 계정 값·로그인 토큰은 생성·저장하지 않았다. 새 테스트·v1→v2·CLI·d1 토큰 저장·검증 자원 종료와 보고서 위치는 [4A 실행 근거](operations.md#단계-4a-실행-근거)에 집약한다. 의도적인 감사 실패 trigger가 남은 격리 fault DB는 서비스용으로 실행하지 않는다.


## 단계 4 추론 설정과 격리 OMP 검증

**단계 4 구현·로컬 실행 검증 완료.** 공개 모델은 `d1-2b-q4`, API는 `typesafe`다. `liquid-systemone`으로 설정하지 않으며 OMP Base URL에는 `/v1`을 넣지 않는다. 모델 목록과 판단 요청 모두 Gateway Bearer 인증·공유 키별 제한·사용량 기록을 거친다.

- Gateway의 `serve --inference-url`은 필수이며 origin URL을 지정한다. 경로가 붙은 API URL을 대신 넣지 않는다.
- 내부 추론 키는 `PICKSY_INFERENCE_API_KEY` 환경변수로만 전달한다. 외부 클라이언트 키와 분리하며 실제 키를 명령 예시·설정 파일·보고서·로그에 남기지 않는다. `.env` 자동 로딩을 가정하지 않는다.
- 요청·응답 크기와 자원 제한의 기본값은 [API 계약](../api-contract.md#자원-제한과-오류)을 따른다. 아래 확인된 수치 CLI는 모두 양수여야 한다. admission은 본문 읽기 전에 동시 추론 수+대기열 수의 총 수용량을 즉시 확보하며, 대기열과 추론의 deadline은 별개다.
- Gateway는 추론 재시도·redirect 추적·proxy 환경변수 사용을 하지 않는다. readiness는 DB+upstream `/health`를 총 500ms 안에 확인하며 liveness는 독립적이다. 연결할 수 없는 loopback upstream에서 추론 502·ready 503·live 200을 실제 확인했다.
- HTTP future drop의 guard가 사용량 499 기록·permit 반환을 처리한다. 실제 OMP 연결 취소 후 DB 499·pending 0과 후속 정상 추론 200을 확인했다. DB 장애·런타임 강제 종료에는 pending이 남을 수 있다. 클라이언트에 499 응답 전달이나 엔진 내부 CPU 계산의 즉시 중단 시간은 보장하지 않는다.

다음은 확인된 CLI로 기본 제한값을 지정하는 기동 예시다. 먼저 로컬 DB를 준비하고 `PICKSY_INFERENCE_API_KEY`를 안전하게 환경에 주입한다. `INFERENCE_ORIGIN`은 실제 내부 서버의 origin을 넣는 예시용 셸 변수이며 Gateway가 자동으로 읽는 설정 키가 아니다. 실제 주소·키를 문서나 로그에 출력하지 않는다.

```sh
./target/debug/picksy --db .local/picksy-dev/gateway.sqlite3 serve \
  --bind 127.0.0.1:18081 \
  --inference-url "${INFERENCE_ORIGIN:?내부 추론 origin을 설정하세요}" \
  --inference-concurrency 1 \
  --inference-queue 8 \
  --queue-timeout-ms 500 \
  --inference-timeout-ms 8000 \
  --body-timeout-ms 1000
```

`scripts/omp-probe.ts`는 실제 설치 OMP 18.19.0의 `NativeJudge`·`fetchTypeSafeModels`로 모델 목록·합성 정상·오류·취소를 검증했다. 다음은 당시 명령의 구조다. `PICKSY_REPO`는 저장소 절대 경로를 넣는 예시용 셸 변수다.

```sh
/usr/local/Cellar/omp/18.19.0/bin/omp models ls \
  --kind judge --json --no-extensions \
  -e "${PICKSY_REPO:?저장소 절대 경로를 설정하세요}/scripts/omp-probe.ts"
```

실행 전 별도 임시 HOME·cwd·`PI_CODING_AGENT_DIR`와 환경변수 허용 목록을 준비한다. 기존 세션·profiles·auth는 변경하지 않으며 위 명령만으로 환경 격리가 자동 생성되는 것은 아니다. 다음 값만 합성 검증용으로 전달하고 키는 메모리에만 둔다.

| 환경변수 | 의미 |
|---|---|
| `PICKSY_PROBE_BASE_URL` | Gateway origin. `/v1`을 붙이지 않는다. |
| `PICKSY_PROBE_KEY` | 합성 검증용 Gateway 키. 파일·로그·명령행에 원문을 저장하지 않는다. |
| `PICKSY_PROBE_MODEL` | `d1-2b-q4` |
| `PICKSY_PROBE_MODE` | 기본 `normal`, 취소 검증은 `cancel` |

실제 검증은 유료 호출 없이 정상 모델 목록·3종 판단·401/400 오류·50ms AbortSignal 취소를 확인했다. 격리 회귀 테스트와 실제 d1 결과, 지연 비교의 한계는 [계획서 결과](../plans/implementation-plan.md#단계-4-결과)에 둔다. 역할 자동 선택·ChainJudge fallback은 검증하지 않았다.

메타데이터 보고서는 `.local/picksy-dev/stage4-ce4ada0a10/report.json`이다. 검증 Gateway 3개와 d1 컨테이너는 정상 종료했고 시험 키 3개는 폐기했다. 로그·DB/WAL의 원문 키 미노출과 pending 0을 확인했으며 파일·컨테이너 자동 삭제는 하지 않았다. 단계 5의 Caddy 제품 풀·복제·운영 검증을 대신하지 않는다.
