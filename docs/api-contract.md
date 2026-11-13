# Picksy TypeSafe API 계약과 검증 경계

[PRD](prd.md) · [추론 Wiki](wiki/inference.md) · [단계별 계획](plans/implementation-plan.md)

## 범위와 근거 수준

현재 채택은 d1-2B Q4_K_M과 공식 llama.cpp HTTP 서버다. 고정 모델·이미지·환경은 [추론 Wiki](wiki/inference.md)를 따른다. **단계 4·4A~4C의 구현·로컬 검증을 완료했다.** 실제 추론·공통 데이터·관리자 인증과 React 화면의 검증 범위는 [작업계획서](plans/implementation-plan.md)에 구분한다. 4C에서는 호스트 Rust의 실제 Google 화면 흐름과 amd64 컨테이너의 정적 제공·API 경계를 검증했다. 컨테이너의 실제 Google 로그인·제품 풀·ARM64·운영 배포는 미검증이다.

## 현재 d1 HTTP 관측과 의미

- 공식 `POST /v1/systemone`에서 텍스트 state와 질문 ID별 choice·noul·score를 실제 호출했다. 제품 범위는 텍스트와 JSON state이며 이미지·assets·box 지원은 가정하지 않는다.
- choice는 선택 ID와 선택지 확률을 반환한다. noul은 true 확률이며 boolean으로 변환하지 않는다.
- score는 낮음부터 높음 순서의 0-based 등급에서 **누적확률이 0.5에 처음 도달하는 중앙값 등급**이다. 확률 가중 평균이나 임의의 0..1 점수로 변환하지 않는다.
- confidence는 최대 확률이다. 정답 확률·다른 엔진의 보정값과 같다고 가정하지 않는다. 질문 ID·선택지·등급 순서·확률 의미를 보존한다.
- 기존 단일 서버 24건은 모두 HTTP 200·출력 토큰 0이었다. 단계 4에서는 내부 model 경로를 확인한 뒤 `d1-2b-q4`로 매핑하고 실제 usage를 보존하는 Gateway 경로를 검증했다. 누락 usage·토큰·비용은 합성하지 않는다.
- 단일 서버의 인증 없는 모델 목록 401·`/health` 200 관측과, 단계 4의 인증된 Gateway 모델 목록·판단 호출 검증은 별도 근거다.

근거: [고정 서버 소스](https://github.com/ggml-org/llama.cpp/blob/5d4c8eeb2/tools/server/server-decision.cpp)의 `format_answer`, `get_decision_confidence`, [현재 실행기](../experiments/d1/evaluate.py), [실행 절차](../experiments/d1/README.md).

## Gateway 인증·DB·상태 계약

단계 3의 인증·DB 기반에 단계 4 추론 경로와 준비 상태를 연결했다. 실행 검증 상태는 [작업계획서](plans/implementation-plan.md), 운영자 명령은 [로컬 개발](wiki/local-development.md)을 따른다.

| 경로 | 현재 동작 |
|---|---|
| `GET /health/live` | 인증 없는 생존 확인, 200. 모델·DB 준비를 보장하지 않는다. |
| `GET /health/ready` | 인증 없는 DB+upstream health 확인, 정상 200·실패 503. 총 확인 기한 500ms. |
| `GET /v1/auth/check` | Bearer 인증·공유 키별 한도·사용량 접수/완료 후 본문 없는 204. 추론을 수행하지 않는다. |
| `GET /v1/models`, `POST /v1/systemone` | Bearer 인증·공유 quota·사용량 기록을 적용하는 모델 목록·실제 추론 경로. |

- 키 형식은 `pk_`와 OS 난수 32바이트의 소문자 16진수 64자리다. 키 ID는 별도 UUID이며 원문 키는 CLI 또는 관리자 생성 응답에서 한 번만 제공한다. 관리 HTTP는 아래 4B의 별도 세션 인증 계약을 따른다.
- 인증 누락·형식 오류·중복 Authorization·알 수 없는 키·폐기 키는 동일하게 401과 `{"error":"unauthorized"}`를 반환한다. Bearer scheme은 대소문자를 구분하지 않는다.
- 키별 기본 한도는 서비스 전체 60회/분이며 발급 시 지정한다. 초과는 429와 `{"error":"rate_limited"}`, 다음 고정 분 구간까지의 초 단위 `Retry-After`를 반환한다. 두 Gateway가 같은 SQLite를 사용해야 같은 한도를 공유한다.
- DB 인증·제한·접수·완료 기록 실패는 503과 `{"error":"unavailable"}`다. DB 장애를 성공이나 빈 사용량으로 바꾸지 않는다. 사용량 완료는 응답 전 DB 기록이며 클라이언트 수신 확인은 아니다.
- 응답에는 서버가 만든 `x-request-id`와 `Cache-Control: no-store`를 붙인다. 클라이언트가 보낸 요청 ID는 신뢰하지 않는다. 로그에는 요청 ID·상태·소요 시간만 남기고 URI·헤더·본문을 기록하지 않는다.
- 폐기는 이후 DB 검사를 거치는 요청에 반영한다. 이미 접수된 요청은 완료할 수 있다. 초기 생존/준비 경로의 공개 상태에는 내부 주소·DB 내용·모델 정보가 없다.

### 단계 4A 키·사용량 확장 계약

4A 당시 Picksy Server의 공통 DB·CLI 기반을 구현하고 로컬 검증했다. 당시 관리자 HTTP 경로는 추가하지 않았으며 기존 추론 Bearer 인증·모델·score·확률 계약을 유지했다. 후속 관리 HTTP의 현재 계약은 아래 4B 절을 따른다.

- 키 생성은 선택적 `name`을 받되 UUID 식별자와 OS 난수 256비트 비밀키를 서버가 자동 생성한다. 이름은 trim 후 비면 NULL, 최대 128 Unicode scalar 문자, 제어문자 금지, 중복 허용이다. 추론 입력 ID의 UTF-8 바이트 제한과 다른 기준이다. 반환 `name`은 선택 값이고 `display_name`은 이름이 없으면 UUID다. 이름·UUID만으로 인증할 수 없다.
- 공통 `create_key`·`revoke_key`는 행위자 `Cli` 또는 `Admin { session_id: UUID }`를 받는다. 관리 행위자의 신뢰 검증은 관리 세션 경계의 책임이며 세션 UUID 자체가 인증 수단은 아니다.
- 키 생성·폐기와 `admin_events` 기록을 하나의 `BEGIN IMMEDIATE` 트랜잭션으로 처리한다. 감사 기록 실패 시 변경도 롤백한다. 이미 폐기된 키는 기존 폐기 시각을 유지하고 추가 감사 이벤트를 만들지 않는다. 원문 키는 생성 직후 한 번만 제공한다.
- 요청 종류는 실제 인증된 route에서 결정한다. `/v1/systemone`은 `inference`, `/v1/models`는 `models`, `/v1/auth/check`는 `auth_check`다. 요청 헤더·query·본문으로 종류를 지정할 수 없다. 기존 NULL 종류는 `unknown`으로 조회하며 추론으로 추정하지 않는다. `admin` 분류값은 유지하지만 관리 HTTP를 추론 사용량으로 기록하지 않는다.
- 검증된 실제 추론 성공의 input/output 토큰만 완료 상태·소요 시간과 한 번에 저장한다. 취소·429·502·504·기타 미수집 값은 NULL이며 0으로 합성하지 않는다. `i64` 범위를 넘는 upstream 토큰은 안전한 502로 처리하고 반올림·포화하지 않는다. 완료 DB 실패의 503·pending 보존 원칙은 유지한다.
- `UsageFilter`는 선택적 키·UTC Unix 초 `from`/`until`·요청 종류를 받으며 기간은 `[from, until)`다. 음수 시각과 `from >= until`은 거절한다. 폐기된 키의 이력은 조회 가능하고 없는 키는 오류다.
- 공통 `UsageReport`는 `summary`, `by_day`, `by_kind`다. 기존 합계 접근은 `.summary.total`로 바뀐다. 일별 항목은 UTC `YYYY-MM-DD`·실제 데이터가 있는 날만 오름차순, 종류별 항목은 실제 있는 종류만 안정된 순서다. 전체·일별·종류별은 같은 filter와 읽기 트랜잭션 snapshot을 사용한다.
- 각 summary는 기존 `total`, `pending`, `succeeded`, `rejected`, `failed`, `other_completed`와 `input_tokens`, `output_tokens`, `token_recorded`, `token_unknown`을 제공한다. 토큰 지표는 분류된 inference만 집계한다. 미수집 inference는 `token_unknown`, 기존 종류 NULL은 별도 unknown 요청이므로 미수집 inference로 추정하지 않는다. 수집값이 하나도 없으면 토큰 합계는 NULL이고 실제 수집한 0은 0이다. 정수 overflow는 오류이며 가짜 0·포화 합계를 반환하지 않는다.

CLI 출력·옵션은 [로컬 개발](wiki/local-development.md#단계-4a-키이름사용량-cli), 스키마·Migration·감사 데이터와 검증 결과는 [DB와 배포](wiki/operations.md#단계-4a-스키마-확장)에 둔다. 필수 스키마 검사는 새 열·테이블을 포함하며 기존 DB를 자동 Migration하지 않는다. 구 바이너리가 v2 DB를 새로 열 수 없다는 엄격한 Migration 경계는 유지하며 무중단 호환을 보장하지 않는다.

### 단계 4B 관리자 인증·관리 HTTP 계약

관리 API는 추론 Bearer 인증과 별도의 관리자 쿠키 세션을 사용한다. API 키는 관리자 권한을 부여하지 않으며 관리자 쿠키도 추론 API 키를 대신하지 않는다. 실제 Google 로그인·허용 계정 불일치 거부·관리 HTTP의 로컬 검증을 완료했고 4C에서 React 화면을 연결했다. 실행 근거는 [운영 문서](wiki/operations.md)와 [작업계획서](plans/implementation-plan.md)에 둔다.

| 경로 | 계약 |
|---|---|
| `GET /auth/google/login` | Google OIDC 로그인 시작, 303 redirect. |
| `GET /auth/google/callback` | 검증 후 세션 쿠키를 발급하고 고정된 `/console`로 303 redirect. 4C에서 4B의 성공 JSON 응답을 대체했으며 오류 JSON·상태는 유지한다. |
| `GET /admin/session` | 200 `{"authenticated":true,"csrf_token":"…","expires_at":…}`. 만료는 UTC Unix 초이며 미인증은 401. |
| `POST /admin/logout` | 세션 무효화·쿠키 만료 후 204. 변경 요청 보호 적용. |
| `GET /admin/keys` | 200 키 정보 배열(`id`, `name`, `display_name`, `created_at`, `revoked_at`, `requests_per_minute`). 원문 키·해시는 제외. |
| `POST /admin/keys` | JSON `{ "name": "선택적 이름", "requests_per_minute": 60 }`. 두 필드는 선택 사항이며 한도 기본값은 60. 201에 `id`, `api_key`, `name`, `display_name` 반환. 원문 키는 이때 한 번만 제공. |
| `DELETE /admin/keys/{id}` | UUID로 논리 폐기 후 204. 이미 폐기된 키도 204이며 감사 중복 생성 없음. 키·사용량·감사 행은 보존. |
| `GET /admin/usage` | 선택적 `key_id`, `from`, `until`, `kind` query로 기존 공통 집계 조회. |

- 변경 요청은 유효한 세션 Cookie와 `X-CSRF-Token`, 설정된 origin과 정확히 일치하는 `Origin`을 모두 요구한다. 추론 Bearer만으로 관리 API를 호출할 수 없다.
- Google 로그인은 `openid email` 최소 범위로 서버에서 코드 교환한다. state·브라우저 결합·nonce·PKCE와 ID 토큰의 서명·issuer·audience·만료를 검증하고 `email_verified=true` 및 비공개 설정의 허용 계정 일치를 요구한다. 공식 근거는 [Google OpenID Connect](https://developers.google.com/identity/openid-connect/openid-connect)다.
- 키 이름·UUID·난수 비밀키 정책과 생성/폐기·감사의 DB 원자성은 4A 계약을 유지한다. 세션 식별자만 감사 행위자로 남기며 이메일·원문 키·로그인 토큰·요청/결과 본문은 감사·로그에 복제하지 않는다.
- usage의 기간은 UTC Unix 초 `[from, until)`이며 `kind`는 `inference`, `models`, `auth_check`, `admin`, `unknown`을 구별한다. 미지정 시 전체 종류다. 기존 `summary`·`by_day`·`by_kind`, NULL/실제 0 구분·동일 snapshot 집계 계약을 유지한다. 관리 API 호출은 추론 사용량·토큰 통계에 넣지 않는다.
- 기존 추론 body 제한·timeout·오류 처리와 DB 실패 차단은 완화하지 않는다. 환경 주입은 [로컬 개발](wiki/local-development.md#google-로그인-개발-설정), 공유 DB·세션·Migration 경계는 [운영 문서](wiki/operations.md)를 따른다.
- 구현 근거는 [src/admin.rs](../src/admin.rs)의 `AdminConfig`, `router`, `authenticate`, `google_identity`와 정확 버전 지정·`Cargo.lock`에 잠긴 `openidconnect` 4.0.1이다. 로그인 상태는 600초, 세션은 28,800초다. state를 DB에서 코드 교환 전에 원자적으로 소비하며 이후 취소·Google 오류에도 재사용하지 않는다. nonce·PKCE·브라우저 결합은 난수 브라우저 seed와 설정 fingerprint로 도메인을 분리해 도출한다.
- HTTPS 쿠키는 `__Host-picksy_admin`·`__Host-picksy_login`, HTTP loopback 개발 쿠키는 `picksy_admin`·`picksy_login`이다. 모두 `Path=/; HttpOnly; SameSite=Lax`이며 HTTPS에는 `Secure`를 붙인다. 세션/CSRF 원문은 DB에 저장하지 않고 해시로 검증한다. CSRF는 세션 토큰에서 도출하여 세션 JSON으로 제공한다.
- 세션은 매 요청 DB에서 만료·폐기·허용 이메일 해시·설정 fingerprint를 확인한다. 네 설정값 변경 후 재시작하면 기존 세션과 진행 중 로그인을 거부한다. 같은 허용 이메일의 Google `sub` 변경은 기존 세션이 폐기됐어도 자동 승인하지 않는다.
- 공유 고정 UTC 분 제한은 login 30회, callback 60회, admin 300회다. 관리 quota는 유효 세션 인증과 변경 요청의 CSRF·Origin 검증 후, callback quota는 유효 state·nonce·브라우저/설정 결합·만료를 확인하고 상태를 원자 소비한 후 차감한다. callback이 429로 거절돼도 소비한 state는 재사용할 수 없다. 인증 실패 요청이 이 두 quota를 소진하지 않도록 분리한다. login은 query를 거절한 뒤 한도를 차감한다. 초과는 429 `rate_limited`와 `Retry-After: 60`이며 추론 키별 quota와 별개다. 공개 login 버킷은 전역 공유이므로 익명 요청이 로그인 시작 한도를 소진할 수 있는 잔여 DoS 위험이 있다.
- 관리자 URI 4096 bytes·헤더 합계 16384 bytes·body 4096 bytes·body 수신 5초 제한을 적용한다.
- 관리자 오류 JSON의 `error`는 400 `invalid_request`, 401 `unauthorized`, 403 `forbidden`, 404 `not_found`, 408 `request_timeout`, 413 `request_too_large`, 429 `rate_limited`, 503 `unavailable`로 구분한다. 응답은 `Cache-Control: no-store`, `Referrer-Policy: no-referrer`를 사용한다. 관리 설정 미지정·DB 실패를 성공이나 빈 목록으로 바꾸지 않는다.
- usage의 `kind=all`은 HTTP 값이 아니다. 전체 조회는 `kind`를 생략한다. 잘못된 UUID·기간·종류는 400, 없는 키는 404이며 폐기된 키의 이력은 조회할 수 있다.

### 단계 4C 관리자 정적 화면

- `GET /console`은 사용량, `GET /console/keys`는 키 관리 React 화면이다. `/`는 `/console`로 이동한다. 화면 shell만 공개되며 실제 데이터·변경은 4B 관리 API 인증을 통과한다.
- 자산은 `/console/assets/app.js`·`/console/assets/app.css`만 제공한다. 포괄적인 SPA fallback이나 디렉터리 탐색을 제공하지 않으며 `/admin/*`·`/auth/*`·`/v1/*`의 오류는 HTML로 변환하지 않는다.
- 파일 누락 시 해당 화면·자산은 JSON 503이며 추론 전용 서버 기동을 막지 않는다. 지정하지 않은 자산·경로는 404다. 정적 루트·자산 디렉터리·파일의 symlink를 차단하고 일반 파일만 읽는다.
- 정적 응답은 실제 MIME, `X-Content-Type-Options: nosniff`, `Cache-Control: no-store`, `Referrer-Policy: no-referrer`와 CSP를 적용한다. 스크립트·API 연결은 동일 출처만 허용하며 프레임 삽입을 금지한다. Radix의 동적 스타일을 위해 `style-src`에만 `unsafe-inline`을 허용한다.
- 기간·키·종류 필터는 기존 usage 계약을 사용한다. 원문 키는 발급 응답 직후 화면 메모리에서만 표시·복사하고 닫기·이동·로그아웃 이후 재조회하지 않는다. 서버 API의 비밀키 재조회 경로를 추가하지 않았다.





## 단계 4 구현 계약과 검증 범위

아래는 구현된 계약이다. 실제 d1 정상·오류 시나리오와 제어 upstream 기반 격리 회귀 테스트를 구분한다. 제품 풀·운영 환경에 대한 보장은 아니다.

### 공개 경로와 모델 목록

- `GET /v1/models`와 `POST /v1/systemone` 모두 Bearer 인증·공유 키별 quota·사용량 기록을 적용한다.
- 공개 모델 ID는 `d1-2b-q4`로 고정한다. 모델 목록은 아래 구조를 사용한다. 설명 문자열은 예시이며 `release_date`는 확인하지 않은 날짜를 만들지 않고 빈 문자열로 둔다.

```json
{
  "models": [
    {
      "name": "d1-2b-q4",
      "description": "d1-2B Q4_K_M 텍스트 판단 모델",
      "release_date": ""
    }
  ]
}
```

### 판단 요청

`POST /v1/systemone`은 JSON 요청을 받는다. 질문 ID·선택지·score 등급의 순서를 보존한다.

| 항목 | 구현 계약 |
|---|---|
| `model` | 필수, `d1-2b-q4`와 정확히 일치해야 한다. |
| `state` | 문자열·JSON 객체·배열. 그 외 타입은 지원하지 않는다. |
| `questions` | 질문 ID를 키로 하는 객체, 1~8개. |
| ID | 질문·선택지 ID는 UTF-8 1~128 bytes. |
| `instructions` | 질문별 문자열, UTF-8 1~4096 bytes. |
| `choice` | `criteria`는 선택지 2~16개의 객체. 각 값은 문자열 또는 null이다. |
| `noul` | `criteria`는 선택 사항이며 제공할 때 true/false 의미의 문자열 설명을 사용한다. |
| `score` | `criteria`는 낮은 등급부터 높은 등급 순서의 문자열 배열, 2~10개. 고정 llama.cpp의 d1 파서 상한을 따른다. |

알 수 없는 필드·타입과 images·assets·box는 지원하지 않으며 조용히 무시하지 않는다. 길이 기준은 문자 수가 아니라 UTF-8 바이트 수다.

score 상한 근거는 [고정 서버 파서](https://github.com/ggml-org/llama.cpp/blob/5d4c8eeb2/tools/server/server-decision.cpp#L311-L319)다. 상류가 거절하는 11개 이상을 정상 입력으로 광고하지 않는다.

### 내부 요청과 응답 의미 보존

- 외부 요청은 Gateway 인증을 거친다. 클라이언트 Authorization·임의 라우팅 헤더를 내부 서버에 그대로 전달하지 않는다.
- 내부 요청에는 `model`을 생략한다. 응답의 실제 model이 `/models/d1-2B-Q4_K_M.gguf`인지 확인한 뒤 공개 ID로 매핑한다. 다른 모델을 요청 모델명으로 덮어써 숨기지 않는다.
- 질문 ID·선택지 ID·답 타입·확률·legend를 검증한다. finite·범위·선택지 일치와 d1의 score 중앙값·confidence 최대 확률 의미를 유지하며 임의 보정하지 않는다.
- 실제 usage를 보존한다. 누락 토큰·비용·답을 만들지 않고 내부 routing·파일 경로를 외부에 노출하지 않는다.
- upstream 자동 재시도·redirect 추적·proxy 환경변수 사용은 하지 않는다.

### 자원 제한과 오류

| 항목 | 기본값 |
|---|---:|
| 요청 body | 64 KiB |
| upstream 응답 | 256 KiB |
| 동시 추론 | 1 |
| 대기열 | 8 |
| 대기열 대기 | 500ms |
| 추론 timeout | 8초 |
| body 수신 timeout | 1초 |

CLI는 `--inference-concurrency`(기본 1), `--inference-queue`(8), `--queue-timeout-ms`(500), `--inference-timeout-ms`(8000), `--body-timeout-ms`(1000)이며 모두 양수여야 한다. 요청·응답 크기의 CLI flag는 추가로 가정하지 않는다. admission은 본문을 읽기 전에 동시 추론 수와 대기열 수를 합한 총 수용량의 자리를 즉시 확보한다. 대기열 대기와 추론에는 별도 deadline을 적용한다. 실제 과부하·timeout과 격리 body·admission 취소 시나리오를 검증했다.

| 상황 | HTTP 상태 |
|---|---|
| 입력 오류·요청 크기 초과·미지원 미디어 타입 | 각각 400·413·415 |
| 인증 실패 | 기존 401 |
| 키별 quota 초과 | 기존 429와 해당 분 구간의 `Retry-After` |
| DB 실패 | 기존 503 |
| 과부하 | 503, `Retry-After: 1` |
| 추론 timeout | 504 |
| upstream 실패·잘못된 응답 | 502 |

오류를 정상 판단이나 빈 answers로 바꾸지 않는다. HTTP future drop 시 guard가 사용량 상태 499 기록과 permit 반환을 처리한다. 실제 OMP의 50ms AbortSignal 취소 뒤 DB 499 1건·pending 0 및 후속 정상 추론 200을 확인했다. 클라이언트에 HTTP 499를 전달하거나 엔진 내부 CPU 계산이 즉시 중단됨을 보장하지 않는다. DB 장애·런타임 강제 종료에는 pending이 남을 수 있다.

### 생존·준비 상태의 구현 경계

readiness는 DB와 upstream `/health`를 총 500ms 이내에 확인하며 liveness는 이 의존성과 독립적이다. 실제 d1 `/health`의 인증 없는 200을 확인했다. 연결할 수 없는 loopback upstream에서 실제 Gateway는 추론 502·ready 503·live 200을 반환했다.

### 설정과 OMP 검증 경계

- `serve`의 `--inference-url`은 필수이며 origin URL이어야 한다. 내부 인증 키는 `PICKSY_INFERENCE_API_KEY` 환경변수로만 받으며 명령행·문서·로그에 원문을 남기지 않는다.
- OMP의 API 형식은 `typesafe`이며 `liquid-systemone`이 아니다. Base URL에는 `/v1`을 포함하지 않는다.
- OMP 18.19.0의 실제 `NativeJudge`에서 d1 중앙값 score 소비를 확인했지만 `ScoreAnswer` 문서는 기대값으로 설명한다. wire 수용과 의미 동일성을 구분하며 무보정 계약을 명시한다.
- `scripts/omp-probe.ts`의 실제 `NativeJudge`·`fetchTypeSafeModels`로 모델 목록·3종 판단·오류·취소를 검증했다. 기존 설정·인증·세션을 건드리지 않는 격리 실행이며 명령은 [로컬 개발](wiki/local-development.md#단계-4-추론-설정과-격리-omp-검증)에 둔다. 역할 자동 선택·ChainJudge fallback·Caddy 제품 풀·ARM64·운영 배포는 이번 범위 밖이다.
- [이전 계약·OMP 조사 이력](wiki/history/2026-11-13-api-contract-history.md)의 과거 엔진별 상한·오류·confidence는 현재 계약에 이식하지 않는다.
