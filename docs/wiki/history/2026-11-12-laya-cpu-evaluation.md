# Laya CPU 검증과 품질 재검토

[결정 인덱스](history.md) · [현재 추론 지식](../inference.md) · [API 계약](../../api-contract.md)

- 기록일: 2026-11-12
- 결정일: 기술 검증은 2026-11-12. 제품 모델의 최종 채택은 미확정.
- 상태: 기술 검토 결과 기록 · 품질 위험의 처리 방향은 사용자 판단 필요.
- 출처: 사용자의 단계 1 실행 요청, 공식 고정 소스·모델 카드, 실제 격리 CPU 추론.
- 후속 결정: [2026-11-13 Decider Q4 채택과 단계 2 승인](2026-11-13-decider-q4-adoption.md). 아래 당시 판단·실험 결과는 역사로 보존한다.

## 배경과 선택

Intel Mac에서 운영과 같은 Linux 계열 실행 경로를 확인하기 위해 Docker `linux/amd64` CPU 실험을 선택했다. macOS x86_64의 PyTorch wheel 제약을 피하되 이를 ARM 검증으로 해석하지 않는다.

다국어 체크포인트 하나를 고정하고 원본은 읽기 전용으로 마운트했다. 테스트 입력·기대 답·탐색 통과선은 결과를 보기 전에 작성했으며, 실패 후 바꾸지 않았다. 공식 `laya.serve`를 수정하거나 OMP의 실제 인증·모델 설정을 바꾸지 않았다.

## 결과와 판단

실제 HTTP 추론·인증·오류 경계는 확인했다. OMP와 기본 wire 형식은 맞지만 Laya 자체에는 모델 목록 경로가 없고, 임의 모델명 자동 라우팅과 confidence 의미 차이가 있어 Gateway의 명시적 계약이 필요하다.

소규모 score 평가가 사전 통과선에 미달했다. 높은 confidence를 가진 한국어 choice 오분류도 관측했다. 따라서 Laya가 Jev를 대체한다거나 모든 판단 종류에 적합하다고 선언하지 않는다. score를 제외하거나 다른 모델로 자동 전환하는 범위 변경은 실행하지 않았다. 정확한 수치와 현재 제약은 연결된 추론 문서에서 관리한다.

## 실행 근거와 보존 범위

- 당시 실험 구현은 정리 대상이다. [보존한 입력·실험 기록](../../../experiments/stage1/README.md)과 아래 원본 결과·고정 외부 근거를 참조한다. 아래 명령은 과거 실행 기록이며 현재 checkout의 실행 안내가 아니다.
- 빌드: `docker build --platform linux/amd64 -t picksy-stage1-laya:0.4.31 -f experiments/stage1/Dockerfile .local/picksy-dev/stage1/laya-source`. 의존성 설치와 `pip check` 성공.
- 당시 실행: `docker run`으로 네트워크 차단·호스트 포트 미게시·CPU 4개·메모리 6 GiB·swap 추가 허용 없음. 평가 종료 코드 2는 실행 실패가 아니라 고정 품질 통과선 미달을 뜻한다.
- 원본 합성 응답: 로컬 `.local/picksy-dev/stage1/evaluation-1.json`. Git에는 포함하지 않는다. 다운로드 소스·모델도 `.local/picksy-dev/stage1/`에 보존한다.
- 이미지 ID: `sha256:9930fc042cef4662f10fb90b2bcd6bcd0184d146d46e7db83a356338a96395a3`, 확인된 크기 1,757,748,481 bytes. 베이스는 `python:3.11-slim-trixie@sha256:07d4824036af339fa1b6ef769cc99d9c10a141769538b52b3b597c84f7a7b535`였다.
- 설치 패키지: Laya 0.4.31, torch 2.14.0+cpu, transformers 5.22.2, vegaml 0.16.1, FastAPI 0.162.5, Uvicorn 0.54.0. Vega 모델을 실행하거나 내려받은 것은 아니다.
- 서버는 평가 종료 시 정상 종료했다. 이미지와 종료된 실험 컨테이너 `picksy-stage1-eval`은 자동 삭제하지 않았다. named volume은 생성하지 않았다.

설치된 OMP 18.19.0의 동일 버전 공식 소스를 확인했으나 실제 클라이언트 종단 실행·Oracle ARM·부하·복제본 실험은 이번 결과에 포함되지 않는다.

## 후속 score 대조 평가

사용자의 score 추가 확인 요청에 따라 동일 이미지·모델로 신규 합성 입력 18쌍을 평가했다. 한국어/영어 정상 score, 동일 기준 choice, 순서 반전, 한국어 state와 영어 rubric을 비교하는 총 162개 실제 요청을 수행했다. 기존 3건을 재실행하지 않았으며 학습·보정·모델 교체·API 의미 변경은 하지 않았다.

- [입력 fixture](../../../experiments/stage1/score-cases.json) SHA-256: `6288e54b6e0e483d3dfdb5d9c640a880961dead00da17104263c873635535434`.
- 실행: 최초 실험과 같은 `docker run` 격리·자원 제한에서 이미지 ID를 고정하고 `/experiment/evaluate-score.py`를 실행했다. 컨테이너 이름은 `picksy-stage1-score`.
- 정상 score는 언어별 사전 기준을 모두 통과하지 못해 종료 코드 2를 반환했다. 모든 162개 HTTP 추론·응답 구조·확률/기대값 검사는 통과했다.
- 평가 결과는 `.local/picksy-dev/stage1/score-evaluation-1.json`에 보존했다. 상세 수치와 현재 해석은 [추론 Wiki](../inference.md)에 둔다.
- 관측: 언어 변경만으로 해결되지 않고, 확률 기대값의 중앙 집중과 질문·선택지 순서 민감성이 있었다. 단순한 높은 점수 편향 하나로 원인을 확정하지 않는다.
- 서버는 종료됐으며 이미지·모델·두 번째 종료 컨테이너는 승인 없이 삭제하지 않았다. 단계 2로 진행하거나 제품 범위를 축소하지 않았다.

## 공식 체크포인트 추가 비교

사용자의 다른 체크포인트 비교 요청에 따라 공식 english와 typed-decisions를 추가 다운로드하고 동일 fixture·통과선·이미지·격리 자원 제한으로 순차 평가했다. 영어 기본 모델과 특정 업무 흐름용 영어 모델을 비교하되 한국어 지원을 전제하지 않았다.

- english: `convaiinnovations/laya` 루트, revision `85cc74ddae6109077ebe2853a9b3077fb6606a78`. weights SHA-256 `891102d372688fc2a094dac56a384bc537b87c63f21f9f3dac0be2b7cbc8d86c`.
- typed-decisions: `convaiinnovations/laya-typed-decisions`, revision `e929ae5cf69bc34259cd2f95c9e91145b818b1f0`. weights SHA-256 `4fa56de72383a9d3efa9cfa78955733c81b9fc8067a587ca4beb82c78107a24e`.
- 당시 실행: 기존 이미지 ID에 `python /experiment/evaluate-score.py --checkpoint english` 및 `--checkpoint typed-decisions`. preload·로컬 모델 등록·읽기 전용 마운트를 각각 일치시켰다. 조건은 [보존 기록](../../../experiments/stage1/README.md#공식-체크포인트-비교)에 있다. 삭제된 runner의 현재 실행 가능성을 뜻하지 않는다.
- 토큰 예산은 기존 다국어 기본값인 1024/head 256으로 통일했다. english 원래 기본값과의 차이와 공식 런타임의 `choice:11+` temperature 제한 경고를 현재 지식에 명시했다. 원본 파일이나 판정 기준을 변경하지 않았다.
- 추가 324개 HTTP 응답의 실제 모델·구조·확률 기대값·잘림 없음 검사를 통과했다. english는 영어 score만 통과하고 한국어는 미달했다. typed-decisions는 두 언어 모두 미달했다. 두 명령 모두 전체 품질 기준 미달로 종료 코드 2를 반환했다.
- 보고서: `.local/picksy-dev/stage1/score-english-1.json`, `score-typed-decisions-1.json`. 기존 multilingual 보고서를 포함한 세 결과의 fixture 해시·판정 기준·패키지 버전을 대조했다. [비교 수치와 해석](../inference.md#공식-체크포인트-비교)은 현재 Wiki에 둔다.
- 서버 종료 후 컨테이너 `picksy-stage1-score-english`, `picksy-stage1-score-typed-decisions`와 다운로드 파일을 보존했다. 운영·OMP 설정·제품 모델 계약을 바꾸거나 단계 2를 시작하지 않았다.

## 현재 OMP Jev 직접 대조

사용자가 현재 연결된 Jev와 비교를 요청했다. OMP 설정에서 judge 역할을 확인하고 기존 fixture의 36개 상태에 정상 score·choice 두 질문을 `judge_batch`로 전달했다. 실제 배치 모델은 모두 `typesafe/jev-1.14.0`으로 보고됐으며 72개 판단 모두 성공했다. 추가 재시도는 0으로 지정했다.

- Jev는 두 언어의 score·choice 모두 18/18이었다. 반환 score의 정규화 MAE는 0이지만 내부 확률 보정 의미까지 확인한 것은 아니다.
- 원본과 메타데이터: `.local/picksy-dev/stage1/jev-comparison-1.json`. 보고된 비용 합계 $0.00087549. 실제 사용자 코드·대화·비밀값 대신 기존 합성 상태만 사용했다.
- Python helper 문서와 달리 `judge_batch`가 awaitable이 아니어서 첫 배치 생성 후 TypeError가 발생했다. 완료 알림의 배치 ID로 핸들을 복구해 결과를 회수했으며 해당 요청을 중복 실행하지 않았다.
- [현재 비교와 한계](../inference.md#현재-omp-jev와-직접-비교)를 반영했다. 게임·OMP 실사용 적합성, Laya의 OMP 종단 연동과 ARM 성능을 검증했다고 주장하지 않는다.

## 동일 질문 묶음 지연 측정

동일 질문으로 속도를 비교해 달라는 요청에 따라 기존 36개 상태에 score·choice를 묶어 동시 요청 1개로 측정했다. 첫 Jev 재호출은 캐시 재사용이 의심되어 순수 추론 지연으로 채택하지 않았다. 새 측정 식별자를 포함한 동일 state를 양쪽에 전달한 후속 측정은 Laya 평균 615.62ms, Jev 평균 219.01ms였다. 준비 시간은 제외하고 첫 추론은 포함했다.

- Laya 실행: 기존 이미지 ID·자원·격리 조건에 `/experiment/evaluate-latency.py --measurement-id <동일 UUID>`. 종료 컨테이너 `picksy-stage1-latency-paired`, `picksy-stage1-latency-paired-2`를 보존했다.
- Jev 실행: OMP `judge_batch`, 배치당 6개 상태, `concurrency=1`, `retries=0`, 배치별 순차 호출. 호출 직전과 완료 회수 시점에 단조 시계로 측정했다. 실제 모델 `typesafe/jev-1.14.0`, 36개 상태 모두 성공.
- 원본 `.local/picksy-dev/stage1/laya-latency-paired-1.json`, `jev-latency-paired-1.json`은 보존하되 최종 비교에는 같은 접두사의 `-2.json` 보고서만 사용한다. 두 최종 보고서의 fixture 해시·측정 식별자가 일치한다.
- [현재 결과와 한계](../inference.md#동일-질문-묶음의-대기-시간)에 평균·언어별 품질과 캐시·네트워크·실행 경로 차이를 기록했다. 서버·모델을 변경하거나 ARM 성능을 추정하지 않았다.

## Decider Q8 대체 후보 실행

사용자의 Decider-2B Q8 테스트 요청에 따라 Laya 실행 컨테이너가 없는 것을 먼저 확인하고, 고정 GGUF·전용 라이브러리의 CPU 실험을 수행했다. 기존 이미지 위에 별도 실험 이미지를 빌드했으며 Laya 서버를 함께 실행하거나 OMP 설정을 바꾸지 않았다.

- 공식 GGUF revision `0df001b8fe4e5298c5dd45d875d3795fcdaea0ab`, Q8 SHA-256 `b89b9bed14dc889075de93136e9a9e3cd859c26b008bfb4bb0211ca0d7357c27`. 다운로드 manifest와 두 실행 보고서의 모델 파일 해시 일치.
- `decider-ai==1.10.0`, `llama-cpp-python==0.3.36` CPU 빌드·정상 의존성 설치·`pip check` 성공. CPU 4개·메모리 6 GiB·추가 swap 없음·네트워크 차단·읽기 전용 모델 마운트.
- 원문과 기존 paired2 JSON 상태 각각 36개 요청을 실행했다. 두 조건 모두 기존 score gate 통과, 종료 코드 0·OOM 없음. 품질은 개선됐지만 현재 Intel 환경의 지연은 가벼운 판단 용도에 충분하지 않다. 정확한 수치와 입력 표현 민감성은 [현재 비교](../inference.md#decider-2b-q8-대체-후보)에 둔다.
- 모델 config의 v11 표기와 v12 파일명의 불일치, score의 독립 수준 판단, 확률 반올림, CPU_REPACK 안내를 숨기지 않았다. 별도 보정·파인튜닝·유료 Jev 재호출은 하지 않았다.
- [실험 코드와 이미지 근거](../../../experiments/decider-q8/README.md), 로컬 `.local/picksy-dev/decider-q8/` 결과·로그·모델을 보존한다. 두 실행 컨테이너는 정상 종료됐고 최종 확인 시 실행 중인 컨테이너는 없었다.

## Q4 비교와 승인된 컨테이너 정리

Q4 비교와 사용하지 않는 컨테이너 정리 요청에 따라 기존 Picksy 종료 컨테이너 10개를 명시적으로 삭제했다. 이미지·모델·호스트의 원본 보고서는 삭제하지 않았다. 동일 리비전의 Q4_K_M만 추가해 기존 Q8 이미지에 갱신된 runner를 읽기 전용 마운트하고 원문/paired2 각 36개 요청을 실행했다.

- 원문 한국어 score 17/18·choice 18/18, 영어 모두 18/18. JSON 조건은 두 언어 모두 18/18. 두 조건 모두 기존 score gate 통과.
- 평균 지연은 Q8 대비 약 1.6배 개선됐으나 빠른 CPU 판단 목표는 달성하지 못했다. 전체 비교와 주의점은 [현재 Q4 결과](../inference.md#q4-대조-결과)에 기록했다.
- 기존 결과·입력·기준·temperature는 변경하지 않았다. Q4 결과·로그와 새 종료 컨테이너 `picksy-decider-q4-original`, `picksy-decider-q4-paired2`는 보존하며 실행 중인 컨테이너는 없다.
