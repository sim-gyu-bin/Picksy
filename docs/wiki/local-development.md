# 로컬 개발 환경

[Wiki](wiki.md) · [DB와 배포](operations.md) · [작업계획서](../plans/implementation-plan.md)

## 실행 전제

Intel Mac의 Rust 호스트 타깃은 `x86_64-apple-darwin`, Docker 엔진은 `linux/amd64`다. Oracle 운영 타깃 `linux/arm64`와 구분한다. 확인한 도구 버전과 명령 결과는 작업계획서 단계 0에 기록한다.

Docker CLI와 Compose가 설치되어 있어도 Desktop 엔진이 실행 중이어야 한다. 엔진 소켓 연결이 실패하면 Docker Desktop 실행 상태부터 확인한다. 정상 연결 확인에는 `docker version`, `docker info`, `docker compose ls --format json`을 사용한다. 마지막 명령의 빈 배열은 실행 중인 Compose 프로젝트가 없다는 뜻이며 컨테이너 실행 검증은 아니다.

## 로컬 실험 경계

- Compose 프로젝트 이름은 `picksy-dev`로 한다. 운영은 별도 프로젝트와 별도 볼륨을 사용한다.
- 로컬 Caddy 진입점은 `127.0.0.1:18080`을 예약한다. 직접 Gateway를 실행하는 개발 모드에는 `127.0.0.1:18081`을 예약한다. 실제 실행 전 포트 충돌을 확인한다.
- Compose 풀의 Gateway·Decider 개별 인스턴스에는 고정 호스트 포트를 할당하지 않는다. 내부 Caddy 리스너와 Decider는 호스트에 공개하지 않는다. 컨테이너 내부 서비스 포트는 실제 서버 계약을 확인한 뒤 정한다.
- 로컬 직접 실행의 DB·모델·실험 결과는 저장소의 `.local/picksy-dev/` 아래에 둔다. 실험 모델·결과는 이미 있으며 운영 DB와 혼용하지 않는다.
- Compose DB는 후속 구현에서 프로젝트 범위 named volume을 사용한다. 개발 볼륨은 `picksy-dev` 범위에 속하도록 하고 운영 볼륨을 external volume이나 직접 경로로 연결하지 않는다. 서비스명·볼륨 키·컨테이너 DB 경로는 아직 미정이다.
- 로컬 데이터는 운영으로 복사하지 않는다. 실제 키나 운영 데이터 대신 합성 입력을 사용한다. 볼륨 삭제·초기화·`docker compose down -v`는 별도 승인 없이 실행하지 않는다.

## 설정 예시와 상태

[.env.example](../../.env.example)은 비밀값이 없는 초기 로컬 설정 예시다. `COMPOSE_PROJECT_NAME`은 Compose 변수이고, `PICKSY_DEV_*`는 후속 구성용 예약 변수다. 실제 단계 2 설정이 이를 읽는지는 해당 구현과 검증 결과로 판단하며 예시만으로 적용을 보장하지 않는다.

[.gitignore](../../.gitignore)는 실제 `.env` 계열, `.local/`, 루트 `target/`을 제외하고 `.env.example`은 보존한다. ignore는 비밀값 유출 방지의 보조 장치일 뿐, 이미 추적된 파일이나 로그를 보호하지 않는다.

단계 0에서는 컨테이너·볼륨 생성, 모델 다운로드, Rust 앱 빌드, ARM64 검증을 수행하지 않았다. 실험용 서버 실행은 후속 단계에서 필요한 범위만 진행한다.

## 단계 1 실험 확장

단계 1에서는 [격리 CPU 실험](../../experiments/stage1/README.md)을 실행했다. 소스·모델·합성 응답은 `.local/picksy-dev/stage1/`, 이미지·종료된 컨테이너는 Docker 엔진에 보존한다. 호스트 포트나 named volume은 추가하지 않았다. 준비 단계 0의 미실행 기록과 이후 실제 실행 결과를 구분한다.

현재 채택은 [Decider Q4](history/2026-11-13-decider-q4-adoption.md)다. `.local/picksy-dev/decider-q8/`의 Q8/Q4 모델·보고서는 경로를 유지한다. Laya·Kev·Jeff 실험 소스 정리 승인은 `.local` 데이터나 Docker 이미지·컨테이너·볼륨 삭제 승인이 아니다.
