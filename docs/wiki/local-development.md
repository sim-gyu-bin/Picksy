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

## 현재 추론과 과거 실험

현재 채택은 [d1 Q4](history/2026-11-13-d1-q4-adoption.md)다. 고정 모델·이미지·실측은 [추론 Wiki](inference.md), 실행기는 [experiments/d1](../../experiments/d1/README.md), 공통 입력은 [short-cases.json](../../experiments/fixtures/short-cases.json)에 둔다. `.local/picksy-dev/d1-q4/` 모델·보고서는 보존한다.

과거 모델·실험 원본·전용 Docker 자원은 [채택 기록의 승인 범위](history/2026-11-13-d1-q4-adoption.md#승인된-정리-범위와-보존)에 따라 정리 대상이다. 과거 수치·실패·해시·출처는 [비교 History](history/2026-11-13-inference-comparison.md), 풀 구성은 [풀 History](history/2026-11-13-pool-validation.md)에 남긴다. 문서의 정리 승인은 실행 완료 보고가 아니며 Rust 코드·DB·History·공용 이미지는 보존한다. 단계 4 진행 승인·미구현과 운영 미승인을 구분한다.

## Rust Gateway 실행

루트의 단일 `picksy` crate를 사용한다. 현재 검증 도구체인은 Rust 1.95.0이며 CI도 같은 버전을 설치한다. 설정은 CLI로만 받으며 `.env`와 `PICKSY_DEV_*`, `RUST_LOG`를 읽지 않는다.

```sh
cargo build --locked --bin picksy
# 로컬 DB만 생성·Migration한다. 운영에서는 승인된 백업·단일 실행 절차가 별도로 필요하다.
./target/debug/picksy --db .local/picksy-dev/gateway.sqlite3 migrate
./target/debug/picksy --db .local/picksy-dev/gateway.sqlite3 key create --requests-per-minute 60
./target/debug/picksy --db .local/picksy-dev/gateway.sqlite3 key list
./target/debug/picksy --db .local/picksy-dev/gateway.sqlite3 serve --bind 127.0.0.1:18081
```

`key create`는 `id`와 원문 `api_key`를 한 번 출력한다. 출력은 로그·문서·저장소에 붙이지 않는다. DB에는 원문이 없으므로 이후 복구할 수 없으며 새 키 발급과 기존 키 폐기를 사용한다. `key revoke <id>`는 폐기 시각을 보존하는 반복 가능한 명령이고 데이터를 삭제하지 않는다. `usage [--key-id <id>]`는 전체/키별 누적 집계를 JSON으로 제공한다.

`serve`·키 관리·사용량 조회는 기존 DB와 Migration 버전·체크섬·필수 열을 검증한다. DB가 없거나 스키마가 다르면 자동 생성·수정하지 않고 실패한다. 생존 `/health/live`, DB 준비 `/health/ready`, 실제 보호 API `GET /v1/auth/check`를 제공한다. [HTTP 계약](../api-contract.md). 추론 경로와 OMP 연결은 단계 4 대상이다.

검사 진입점은 로컬과 [CI](../../.github/workflows/check.yml) 모두 `bash scripts/check.sh`다. 포맷 확인·Clippy 경고 거부·테스트를 수행한다. 테스트는 합성 키와 고유한 `.local/picksy-dev/gateway-tests/<UUID>/` DB를 쓰며 운영·유료 API를 연결하지 않는다. 자동 삭제하지 않으므로 반복 실행하면 로컬 테스트 데이터가 누적된다.

소스는 [CLI](../../src/main.rs), [DB](../../src/db.rs), [HTTP](../../src/gateway.rs), [초기 Migration](../../migrations/0001_gateway.sql), [경계 테스트](../../tests/gateway_boundaries.rs)에 둔다. 초기 Migration·실제 실행 완료 여부는 [작업계획서](../plans/implementation-plan.md)와 [DB와 배포](operations.md)를 따른다.
