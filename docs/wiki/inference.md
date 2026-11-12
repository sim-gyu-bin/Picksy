# 추론과 클라이언트 연동

[Wiki](wiki.md)

## 현재 지식

**Decider-2B Q4_K_M 채택 합의 · 단계 1 선택 완료 · 단계 2 승인 · 품질·지연 미해결.** 2026-11-13 사용자 결정으로 Laya 대신 Decider를 채택했다. 직접 CPU 라이브러리 추론 결과는 아래에 보존하며, HTTP 래퍼·독립 풀·OMP 종단 연동의 구현·검증 완료를 뜻하지 않는다. [채택 결정](history/2026-11-13-decider-q4-adoption.md).

- 고정 모델: `Mapika/decider-2b-GGUF@0df001b8fe4e5298c5dd45d875d3795fcdaea0ab`, `decider-2b-v12-Q4_K_M.gguf`, SHA-256 `e6d3a7534a3abc3a723f8263dc694b86f533214a5bf761474113c66d44018340`.
- 런타임: `decider-ai==1.10.0`, `llama-cpp-python==0.3.36`, 직접 CPU 추론. config·응답의 `2b-v11`/`decider-2b-v11`과 v12 파일명 불일치는 보존한다.
- 한국어 짧은 24건: 게임 choice 4/6, OMP choice 5/6, noul 5/6, score 5/6. 채택은 이 실패와 지연을 해소한 판정이 아니다. 제품 품질·ARM64·운영 승인은 별도다.
- 아래 Laya·Q8·Kev·Jeff 절은 비교 이력이다. 삭제 승인된 실험 소스와 달리 `.local` 원본 결과·모델과 Docker 자원은 이번 정리의 삭제 대상이 아니다.

## 이전 Laya 계약과 선택

- 상세 요청·응답·모델 목록·변환 규칙의 원본은 [초기 API 계약](../api-contract.md)이다.
- 설치된 OMP는 18.19.0이며 동일 버전의 공식 태그 소스로 TypeSafe 계약을 확인했다. 설치 바이너리의 종단 호출은 아직 검증하지 않았다.
- Laya의 `POST /v1/systemone`은 실제 동작하지만 `GET /v1/models`는 404다. Gateway가 OMP의 `{models: [{name, description, release_date}]}` 형태를 제공해야 한다.
- 알 수 없는 단순 모델 이름도 자동 라우팅되는 것을 관측했다. Gateway는 공개 모델 허용 목록과 고정 체크포인트 매핑을 별도로 검사해야 한다.
- `noul`은 true 확률, `score`는 0-based 수준의 확률 가중 기대값이다. choice/score의 `confidence`는 정규화 엔트로피이지 정답 확률이 아니다.
- 원래 응답의 `routing.repo`에는 로컬 경로가 들어갈 수 있다. Gateway의 외부 응답에서는 내부 경로를 노출하지 않도록 명시적으로 정제해야 한다.

## 초기 CPU 관측

[고정 합성 입력과 판정 기준](../../experiments/stage1/README.md)을 결과 관측 전에 작성했다. 한국어·영어 고객 문의 분류, 환불 여부, 한국어 긴급도만 포함하며 일반적인 코딩 판단·게임 행동·장문 품질을 대표하지 않는다.

| 종류 | 정답 / 전체 | 사전 탐색 통과선 | 판정 |
|---|---:|---:|---|
| choice | 11 / 12 | 10 / 12 | 탐색 통과 |
| noul | 4 / 4 | 3 / 4 | 탐색 통과 |
| score | 1 / 3 | 2 / 3 | 미달 |

- choice는 한국어 5/6, 영어 6/6. 한국어 기업용 가격·견적 요청을 billing으로 오분류하면서 `answer_confidence=0.9689`를 반환했다. 높은 confidence만으로 오류를 걸러낼 수 있다고 보장하지 않는다.
- 낮음·보통·높음의 기대 위치 0·1·2에 score가 각각 1.6715·1.8107·1.8624를 반환했다. 변환이나 정규화 오류가 아니라 실제 응답의 기대값이며, 이 실험에서는 높은 수준 쪽으로 치우쳤다.
- CPU 4개·메모리 6 GiB 제한의 Linux amd64 컨테이너, PyTorch `2.14.0+cpu`, transformers `5.22.2`. 실제 device는 cpu, 모델은 multilingual 하나였다.
- 다운로드와 해시 계산을 제외한 서버 시작부터 health까지 24.25초. 서로 다른 입력 19개의 지연 중앙값 159.62ms, 최대 413.92ms. 첫 요청을 포함하며 부하 성능·SLA 측정이 아니다.
- 서버 최고 RSS 2,348,032 KiB, 종료 전 RSS 1,768,644 KiB. 시작을 포함한 서버 CPU 사용 시간 41.319초. Oracle ARM 또는 복제본 두 개의 성능으로 일반화하지 않는다.
- 인증 누락·오류 401, state 누락·JSON 오류 400, 타입·미등록 Hub 모델·토큰 한도 422, state 초과 413을 실제 확인했다.

## score 확대 대조 평가

같은 고정 이미지·모델과 CPU 제한을 유지한 채 [신규 합성 입력](../../experiments/stage1/score-cases.json) 18개의 한국어·영어 쌍을 평가했다. 서로 다른 상태는 36개이고, 조건별 재질문을 포함한 실제 요청은 162개다. 이를 독립 사례 162개로 해석하지 않는다.

세 도메인은 명시 등급 추출·상황별 긴급도·감정이며, 각 수준을 동일한 수로 구성했다. 기존 실패 입력·모델·기준은 바꾸지 않았다. 모든 요청이 실제 추론에 성공했고 `usage.truncated=false`였으며 최대 입력 토큰 관측값은 222였다.

| 조건 | 한국어: 기대 위치 ±0.5 | 영어: 기대 위치 ±0.5 | 한국어 정규화 MAE | 영어 정규화 MAE |
|---|---:|---:|---:|---:|
| 정상 score | 9 / 18 | 10 / 18 | 0.2400 | 0.2202 |
| 같은 기준의 choice 확률 기대값 | 12 / 18 | 14 / 18 | 0.2414 | 0.1660 |
| 한국어 state + 영어 rubric의 score | 10 / 18 | 해당 없음 | 0.2433 | 해당 없음 |

정규화 MAE는 절대 오차를 0..2 범위의 폭 2로 나눈 값이다. 실행 전에 정한 정상 score 기준은 언어별 ±0.5 정확도 80% 이상, 정규화 MAE 0.15 이하, 각 수준 정확도 2/3 이상이었다. 한국어·영어 모두 미달했다.

| 정상 score 도메인 | 한국어 | 영어 |
|---|---:|---:|
| 명시된 우선순위 | 4 / 6 | 2 / 6 |
| 상황별 긴급도 | 2 / 6 | 2 / 6 |
| 감정 | 3 / 6 | 6 / 6 |

### 해석과 한계

- **단순히 한국어만의 문제는 아니다.** 영어 감정은 이 작은 표본에서 잘 맞았으나 영어 명시 등급·긴급도는 미달했다. 영어 rubric으로 바꾼 한국어 score도 충분히 개선되지 않았다.
- **확률 기대값과 분류 정답은 다르다.** 정상 score의 argmax는 한국어 13/18·영어 14/18로 맞았지만, 실제 score 값은 더 자주 미달했다. 예를 들어 한국어의 명시된 높은 우선순위는 최고확률 범주가 높음이어도 score가 1.4196이었다. 응답 값은 실제 확률의 기대값과 일치하므로 argmax나 임의 정규화로 바꾸면 원래 계약을 바꾸게 된다.
- **고정된 높은 점수 편향으로만 설명되지 않는다.** 추가 긴급도 실험에서는 한국어 낮음·보통·높음 평균이 각각 1.1808·1.1510·1.1947로 중앙 근처에 몰렸다. 초기 3건의 높은 수준 편향을 모든 입력에 일반화하지 않는다.
- **순서와 표현에 민감하다.** 같은 의미의 choice 선택지 삽입 순서만 뒤집었을 때 argmax가 한국어 4/18·영어 5/18에서 달라졌다. 역순 score의 argmax는 의미 좌표 복원 후에도 각 언어 6/18에서 달라졌지만, 이 조건은 정상적인 낮음→높음 계약을 깨는 진단용이라 운영 해결책으로 쓰지 않는다.
- 같은 도메인의 서로 다른 정답 수준끼리 비교한 순위는 한국어 32/36·영어 34/36이 일치했다. 이는 점수 자체의 정확성이나 안정된 임계값을 보장하지 않는다.
- 모델 내부의 근본 원인이나 학습 결함을 이 실험만으로 확정하지 않는다. tokenizer 경고는 계속 발생했으며 원본·설치 버전을 바꾸지 않았다. 범용 score 채택 보류 판단은 강화됐다. 다른 체크포인트와의 비교는 아래 후속 평가에 기록하며 보정·학습 효과는 미검증이다.

원본 합성 응답은 로컬 `.local/picksy-dev/stage1/score-evaluation-1.json`에 보존한다. 당시 runner는 정리 대상이며 [고정 입력과 실험 기록](../../experiments/stage1/README.md)을 남긴다. 결과를 보고 기준이나 입력을 바꾸거나 재실행해 통과시키지 않았다.

## 공식 체크포인트 비교

동일한 18개 한국어·영어 쌍, 162개 요청 조건과 사전 통과선으로 공식 english·typed-decisions를 추가 평가했다. 각 모델을 같은 이미지·CPU 4개·메모리 6 GiB에서 순차 실행했다. 기존 multilingual 결과는 재실행하거나 덮어쓰지 않았다. 세 보고서의 fixture 해시·패키지·판정 기준이 같고, 각 162개 응답의 실제 모델 라우팅·구조·확률 기대값·잘림 없음 검사를 통과했다.

| 체크포인트 | 한국어 ±0.5 | 영어 ±0.5 | 한국어 정규화 MAE | 영어 정규화 MAE | 사전 score 기준 |
|---|---:|---:|---:|---:|---|
| multilingual | 9 / 18 | 10 / 18 | 0.2400 | 0.2202 | 두 언어 미달 |
| english | 6 / 18 | 15 / 18 | 0.3618 | 0.1295 | 영어만 통과 |
| typed-decisions | 6 / 18 | 14 / 18 | 0.3385 | 0.1389 | 두 언어 미달 |

- **영어 한정 후속 후보는 english다.** 영어의 수준별 정확도는 낮음 4/6·보통 6/6·높음 5/6으로 세 기준을 모두 통과했다. 다만 긴급도에서는 낮음 두 사례를 보통으로 잘못 판단했다. 합성 18건의 탐색 통과이지 제품 수용이나 Jev 대체 증거는 아니다.
- typed-decisions는 영어 정규화 MAE와 수준별 기준은 충족했지만 전체 정확도 14/18로 80%에 미달했다. 같은 기준의 영어 choice argmax는 18/18이었지만 score와 다른 계약이며 이를 점수 대체 근거로 쓰지 않는다.
- **한국어 score는 해결되지 않았다.** 두 추가 모델은 공식 영어 체크포인트로, 한국어 결과는 지원 범위 밖의 진단이다. 한국어 state에 영어 rubric을 준 경우도 english 6/18·typed-decisions 7/18로 개선책이 되지 못했다. 번역 전처리 품질은 평가하지 않았다.
- 요청 토큰 예산은 기존 다국어 실험과 같은 1024/head 256으로 명시했다. english 기본값 512/192와 다르므로 기본 설정 성능으로 해석하지 않는다. 추가 모델 최대 입력은 393 토큰이었고 잘림은 없었다. 원본 temperature 설정을 보존했으며 모델·tokenizer·학습·보정 차이 중 어느 요소가 개선 원인인지는 분리하지 않았다.
- 두 추가 모델 모두 공식 런타임이 `choice:11+` temperature를 0.10058에서 0.5로 제한한다는 경고를 냈다. 이번 세 선택지에는 해당 항목을 사용하지 않는다. 경고를 숨기거나 모델 파일을 고치지 않았다.
- 최고 서버 RSS는 english 2,849,204 KiB·typed-decisions 2,852,568 KiB였다. 모델 하나씩 실행한 amd64 결과이며 동시 복제본이나 ARM 성능을 보장하지 않는다.

재현 명령과 고정 리비전은 [실험 절차](../../experiments/stage1/README.md#공식-체크포인트-비교), 파일 해시와 보고서 경로는 [검증 기록](history/2026-11-12-laya-cpu-evaluation.md#공식-체크포인트-추가-비교)에 둔다. 제품 모델 목록·OMP 설정·자동 라우팅 정책은 변경하지 않았다.

## 현재 OMP Jev와 직접 비교

현재 OMP의 judge 역할 `typesafe/jev-latest`를 `judge_batch`로 호출했고, 여섯 배치의 실제 모델 표시는 모두 `typesafe/jev-1.14.0`이었다. 기존 fixture의 동일한 36개 상태와 정상 score·choice 지시문/기준을 사용했다. 정답은 호출에 전달하지 않았고, 순서 반전·영어 rubric 조건은 이번 비교에서 제외했다.

| 모델 | 한국어 score ±0.5 | 영어 score ±0.5 | 한국어 choice 정답 | 영어 choice 정답 |
|---|---:|---:|---:|---:|
| 현재 OMP Jev | 18 / 18 | 18 / 18 | 18 / 18 | 18 / 18 |
| Laya multilingual | 9 / 18 | 10 / 18 | 13 / 18 | 14 / 18 |
| Laya english | 6 / 18 | 15 / 18 | 7 / 18 | 17 / 18 |
| Laya typed-decisions | 6 / 18 | 14 / 18 | 9 / 18 | 18 / 18 |

- Jev의 반환 score 정규화 MAE는 두 언어 모두 0이었다. 이는 관측된 반환값이며 내부 보정·반올림 여부나 confidence의 완전한 신뢰성을 보장하지 않는다.
- 36개 상태에 두 질문씩 총 72개 판단을 수행했고 실패는 없었다. OMP 보고 비용 합계는 약 $0.00087549였다. 합성 입력만 전송했으며 설정·인증값은 변경하지 않았다.
- Jev는 OMP helper를 통한 원격 호출, Laya는 저장된 로컬 공식 서버 결과다. API 전송 경로와 질문 묶음 방식이 같지 않아 순수 모델 속도·동일 인프라 성능 비교로 해석하지 않는다. 로컬 Laya를 OMP에 연결한 종단 검증도 아니다.
- **이 표본에서는 Jev가 한국어에서 명확히 우세하다.** 다만 명시 등급·긴급도·감정 사례이지 실제 게임 행동이나 OMP 코드 판단 데이터는 아니다. 사용자가 원하는 가벼운 게임·OMP judge 용도에 충분한지는 해당 업무의 새 평가가 필요하다. Jev와의 일치율만을 정답으로 삼지도 않는다.

원본 반환값과 실제 모델·비용 메타데이터는 `.local/picksy-dev/stage1/jev-comparison-1.json`에 보존한다. fixture는 이전 비교와 동일하며 Laya 추론을 다시 실행하거나 결과를 바꾸지 않았다.

### 동일 질문 묶음의 대기 시간

후속 속도 실험은 양쪽 모두 같은 36개 상태에 score·choice 두 질문을 묶어 동시 요청 1개로 실행했다. 이전 원문을 그대로 Jev에 재요청했을 때 비용 없이 여섯 상태 배치가 6~36ms에 끝나는 캐시 재사용 의심 결과가 나와 제외했다. 새 측정 식별자를 포함한 동일 JSON state로 양쪽을 다시 측정했다.

| 측정 | 로컬 Laya multilingual | 원격 Jev 1.14.0 |
|---|---:|---:|
| 36개 요청의 배치 시간 합계 | 22.162초 | 7.884초 |
| 두 질문 묶음 요청당 평균 | 615.62ms | 219.01ms |
| 한국어 요청당 평균 | 712.84ms | 213.64ms |
| 영어 요청당 평균 | 518.40ms | 224.37ms |

이번 호출 경로에서는 Laya의 대기 시간이 약 2.81배였다. 양쪽 모두 준비 시간 제외·첫 추론 포함이며, 여섯 순차 배치의 완료까지 벽시계 시간을 합산했다. Jev의 뒤늦은 `status.elapsedS`가 아니라 호출 직전부터 결과 회수까지 직접 잰 값이다. 단일 실행의 평균으로 중앙값·p95·동시 처리량을 주장하지 않는다.

Laya는 기존 CPU 4개·메모리 6 GiB·고정 이미지에서 실행했고 36개 응답 모두 잘림 없이 구조 검사를 통과했다. 새 식별자를 붙인 상태에서 한국어 score 9/18·choice 13/18, 영어 score 8/18·choice 13/18이었다. Jev는 두 언어 두 유형 모두 18/18이며 실제 버전·비용 약 $0.00094958이 보고됐다. 이전 원문 결과와 입력 형태가 달라 품질 수치를 섞지 않는다.

이 결과는 로컬 HTTP 대 원격 OMP helper의 실제 대기 시간 비교이지 하드웨어·연결 재사용·서비스 캐시를 통제한 순수 모델 비교가 아니다. Oracle ARM 결과도 아니다. [재현 절차](../../experiments/stage1/README.md#jev와-동일-질문-묶음-지연-비교)와 로컬 `laya-latency-paired-2.json`, `jev-latency-paired-2.json`을 `.local/picksy-dev/stage1/`에 보존한다.

## Decider-2B Q8 대체 후보

공식 GGUF v12 파일을 고정해 CPU 라이브러리 경로로 평가했다. `decider-ai 1.10.0`·`llama-cpp-python 0.3.36`, CPU 4개·메모리 6 GiB·swap 추가 없음·네트워크 차단 컨테이너다. Laya 서버와 동시에 실행하지 않았다. 모델·config·tokenizer의 다운로드 해시와 실행 시 해시가 일치했다.

| 원문 상태 기준 | 한국어 score ±0.5 | 한국어 choice 정답 | 영어 score ±0.5 | 영어 choice 정답 |
|---|---:|---:|---:|---:|
| Decider-2B Q8 | 17 / 18 | 18 / 18 | 18 / 18 | 18 / 18 |
| 기존 Laya multilingual | 9 / 18 | 13 / 18 | 10 / 18 | 14 / 18 |
| 기존 Jev 1.14.0 | 18 / 18 | 18 / 18 | 18 / 18 | 18 / 18 |

Decider는 두 언어 모두 기존 score 탐색 기준을 통과했다. 정규화 MAE는 한국어 0.1019·영어 0.0544다. 한국어 명시 등급 `declared_priority_high_b`에서 기대 2에 score 1.42를 반환했다. score argmax는 두 언어 모두 18/18이지만 기대값과는 다른 지표다. 기존 Laya 정상 score·choice는 별도 요청, Decider와 Jev는 두 질문 묶음이다.

원문 두 질문 묶음의 `system_one` 호출 평균은 13.762초·중앙값 12.721초였다. 로딩 약 5.5초는 제외하고 첫 추론은 포함했다. 프로세스 최고 RSS는 2,654,376 KiB였다. **현재 Intel CPU 설정에서는 품질 개선은 확인됐지만 빠른 판단용으로 채택할 속도는 확보하지 못했다.** ARM이나 다른 빌드·양자화의 성능은 미검증이다.

이전 속도 비교와 동일한 JSON state·측정 식별자를 넣은 `paired2` 조건도 새 프로세스로 36개 요청을 완료했다. 한국어·영어의 score·choice 모두 18/18이었고 score 정규화 MAE는 각각 0.0908·0.0542였다. 라이브러리 호출 평균 21.254초·중앙값 19.821초, 검증 등 부대 작업을 포함한 요청당 배치 평균 21.266초였다. 이전 동일 입력 Laya HTTP 평균 0.616초·Jev helper 평균 0.219초와 실행 경로는 다르지만 현재 Decider CPU 경로의 지연이 크다는 점은 분명하다. 원문과 JSON 상태의 성적 차이는 입력 표현 민감성으로 기록하며 더 좋은 조건만 선택하지 않는다.

두 조건 모두 종료 코드 0, OOM 없음, 총 72개 상태 호출·144개 판단을 완료했다. 모델·라이브러리 설정을 중간에 바꾸지 않았으며 Jev API를 재호출하지 않았다. 두 Decider 컨테이너도 종료돼 실행 중인 Docker 컨테이너는 없다. 단일 실행이므로 반복 변동·동시 부하·순수 choice만의 속도는 미검증이다.

### Q8 CPU 병목 계측

기존 이미지·Q8 가중치·온도·CPU 4개를 그대로 두고 대표 한국어 `declared_priority_low_a`의 원문/paired2 상태에서 두 질문 묶음과 choice 단독을 계측했다. 입력 토큰화·행 구성, `llama_memory_clear`, `llama_decode`에 시간을 기록하는 래퍼만 추가했으며 원래 인자와 반환값은 유지했다.

| 대표 요청 | 전체 호출 | llama_decode 합계 | decode 횟수 | 내부 행별 토큰 |
|---|---:|---:|---:|---|
| 원문 score + choice | 7.642초 | 7.607초 | 4 | 105 / 103 / 105 / 130 |
| 원문 choice만 | 2.294초 | 2.291초 | 1 | 130 |
| JSON score + choice | 9.778초 | 9.768초 | 4 | 150 / 148 / 150 / 175 |
| JSON choice만 | 3.065초 | 3.062초 | 1 | 175 |

- **병목은 네이티브 모델 계산이다.** 네 조건 모두 호출 시간의 99% 이상이 `llama_decode`였다. 행 구성은 0.6~1.9ms, 메모리 초기화 합계는 1.7~7.6ms로 작았다. 이 함수 이름은 llama.cpp API 이름이지 텍스트를 반복 생성했다는 뜻은 아니다.
- **score가 세 번의 독립 계산을 추가한다.** 원본 `isolated_levels=true`에서 3등급 score는 3개 행, choice는 1개 행이다. 설치된 `GGUFEngine.score_shared`는 prefix cache 없이 `score_items`를 호출하며 각 행마다 메모리를 비우고 state부터 재계산한다. HTTP 묶음 하나가 모델 forward 하나라는 가정은 이 경로에 맞지 않는다.
- **CPU 최적화가 전부 꺼진 빌드는 아니다.** 실제 라이브러리에서 AVX·AVX2·FMA·F16C·OPENMP·REPACK 활성, Docker CPU flags에도 대응 기능을 확인했다. `GGML_NATIVE=OFF`만을 지연 원인으로 단정하지 않는다.
- cgroup CPU 제한은 `400000 100000`이며 각 구간 프로세스 CPU 시간/벽시계 시간은 약 4배로 네 CPU를 사용했다. 기록된 throttled 시간은 13~62ms로 해당 구간 수 초 지연의 주된 설명이 아니다. 시스템 전체 부하·호스트 스케줄링 영향까지 제거한 시험은 아니다.
- 원문→JSON은 내부 모든 행을 45토큰 늘렸고 해당 대표 입력에서 시간이 증가했다. 전체 36건 평균과 대표 한 건의 시간을 섞거나 단독 choice 품질 전체를 검증했다고 주장하지 않는다.

이 계측 후 같은 입력·동일 의미의 Q4 대조를 수행했고 결과는 아래에 기록했다. prefix cache 최적화는 이득 가능성이 있지만 현재 GGUF 경로에 구현돼 있지 않다. `isolated_levels=false`나 다중 행 packing은 확률·품질이 달라질 수 있으므로 속도만 보고 켜지 않는다. CPU 연산 내부에서 양자화 커널·행렬곱·대역폭 중 무엇이 지배적인지는 아직 분리하지 않았다. [계측 코드](../../experiments/decider-q8/profile.py), 로컬 `.local/picksy-dev/decider-q8/profile-1.json` 및 `.stderr`가 근거다.

### Q4 대조 결과

사용자 승인으로 기존 종료된 Picksy 실험 컨테이너 10개만 삭제하고 이미지·모델·보고서는 보존했다. 같은 리비전의 Q4_K_M 파일을 추가해 Q8과 같은 이미지·패키지·CPU 제한·fixture·온도·판정 기준으로 두 조건을 각각 36건 순차 실행했다. 변경한 것은 선택한 GGUF 파일이며, 런타임 빌드나 isolated score 의미는 바꾸지 않았다.

| 조건 | Q8 요청 평균 | Q4 요청 평균 | Q4 한국어 score / choice | Q4 영어 score / choice |
|---|---:|---:|---|---|
| 원문 | 13.762초 | 8.521초 | 17/18 · 18/18 | 18/18 · 18/18 |
| 동일 JSON 상태 | 21.254초 | 13.407초 | 18/18 · 18/18 | 18/18 · 18/18 |

Q4는 이번 실행에서 Q8 대비 약 1.6배 빠르고 대기 시간이 약 37~38% 감소했다. 정답 개수는 유지됐고 두 언어 score gate도 두 조건 모두 통과했다. 다만 원문 score 정규화 MAE는 한국어 0.1050·영어 0.0650으로 Q8의 0.1019·0.0544보다 조금 커졌다. 같은 한국어 실패 사례의 score는 Q8 1.42에서 Q4 1.36으로 바뀌었다. 이 작은 표본의 정답 개수 보존을 전반적인 양자화 품질 무손실로 해석하지 않는다.

JSON 조건은 기존 Jev 평균 0.219초와 비교하면 여전히 약 61배의 대기 시간이다. 실행 경로가 다른 한계는 유지되지만 **Q4만으로 현재 CPU에서 빠른 판단 목표를 달성하지 못했다.** 원문/JSON 최고 RSS는 각각 2,531,612 / 2,540,088 KiB였다. 파일 크기 감소가 같은 비율의 전체 프로세스 메모리 감소를 뜻하지 않는다.

추가 72개 요청·144개 판단이 정상 완료됐으며 두 컨테이너는 종료 코드 0·OOM 없음으로 종료했다. Q4 파일 SHA-256과 다운로드 manifest, Q8/Q4의 fixture·패키지·config·실행 옵션 일치를 대조했다. 원본 결과는 `.local/picksy-dev/decider-q8/decider-q4-original-1.json`, `decider-q4-paired2-1.json`과 대응 stderr에 있다. 새 Q4 종료 컨테이너 2개는 남아 있고 실행 중인 컨테이너는 없다. 자동 삭제·유료 Jev 호출·운영 변경은 하지 않았다.

### 짧은 게임·OMP 모사 질문

실제 사용자 로그가 아닌 [신규 한국어 합성 사례 24개](../../experiments/decider-q8/short-cases.json)를 실행 전에 정답과 함께 고정했다. Q4·기존 온도·CPU 4개·메모리 6 GiB는 유지하고 요청마다 질문 하나만 보냈다. 정답은 모델에 전달하지 않았으며 첫 추론을 포함하고 로딩은 제외했다. Jev 재호출·새 비교는 하지 않았다.

| 용도 | 정답 / 전체 | 평균 지연 | 중앙값 |
|---|---:|---:|---:|
| 게임 행동 choice | 4 / 6 | 1.383초 | 1.143초 |
| OMP 변경 분류 choice | 5 / 6 | 1.226초 | 1.080초 |
| OMP 요구 충족 noul | 5 / 6 | 0.794초 | 0.818초 |
| 게임 대응 필요성 score | 5 / 6 | 3.690초 | 3.582초 |

choice·noul은 내부 forward 1회, score는 3회였다. 행 길이는 각각 52~72, 46~50, 54~66토큰이었다. 24개 요청이 완료되고 컨테이너는 종료 코드 0·OOM 없이 종료했다. 프로세스 최고 RSS 2,518,276 KiB. 짧은 질문 하나는 기존 긴 두 질문 묶음보다 빠르지만, 일반적인 프레임 단위 실시간 처리에 충분하다고 판단하지 않는다.

관측된 실패:

- 체력 20 또는 30·물약 없음에서 명시한 후퇴 규칙 대신 공격을 선택했다. 단순 수치 조건·규칙을 맡겨도 정확성을 보장하지 않는다.
- README 오타만 수정한 사례를 docs 대신 fix로 분류했다. 기준의 '기존 오류 수정'이 문서 오류까지 포함한다고 읽힐 여지도 있으므로 모델 오류와 rubric 중첩을 분리해야 한다. 사후에 정답이나 기준을 바꿔 통과시키지는 않았다.
- 오름차순 요구에 결과 `[3,2,1]`을 충족으로 판단하고 `noul=1.0`을 반환했다. 기본 noul 온도 0.05의 날카로운 확률은 정확성 보증이 아니다. 원본 설정은 변경하지 않았다.
- 적 없는 안전 구역 사례의 score가 0.70으로 기대 0의 ±0.5 범위를 벗어났다.

이 표본에서는 속도는 줄었지만 새 업무에 대한 품질은 기존 18건 결과보다 낮았다. 단순 체력·물약·정렬 여부처럼 코드로 정확히 검사할 수 있는 조건은 규칙으로 처리하는 것이 안전하다. 언어 판단은 겹치지 않는 기준으로 별도 평가하고 필요 시 사용자 확인용 보조로 제한하는 것이 현재 권고다. 이 합성 표본을 실제 OMP 종단 수용이나 Jev 동급 판정으로 쓰지 않는다.

실행 코드는 [짧은 질문 runner](../../experiments/decider-q8/evaluate-short.py), 원본은 `.local/picksy-dev/decider-q8/short-q4-1.json`과 `.stderr`, fixture SHA-256은 `3de61796fb6a012e519b031acc4d15a3a7b2b1c94ffc5d925ac63aa32fd6d4d0`이다. noul은 0.5 초과/미만으로 판단하고 정확히 0.5는 오답 처리하며 score는 기존 ±0.5 기준이다.

### Oracle 배포 전 CPU 자원 민감도

로컬은 Intel i7-9750H(물리 6코어·논리 12스레드), RAM 16 GiB이며 Docker VM은 CPU 12개·메모리 약 7.75 GiB다. 기존 짧은 질문 실험의 컨테이너 한도는 CPU 4개분·6 GiB였다. 같은 Q4 이미지·24개 질문·온도·추론 스레드 4개를 유지하고 `--cpus 2`만 적용했다. 이는 Intel CPU 시간 할당량 실험이지 ARM 에뮬레이션이나 Oracle 성능 예측이 아니다.

| 유형 | 기존 CPU 4개분 평균 | CPU 2개분 평균 |
|---|---:|---:|
| 게임 choice | 1.383초 | 2.415초 |
| OMP choice | 1.226초 | 1.639초 |
| OMP noul | 0.794초 | 1.732초 |
| 게임 score | 3.690초 | 6.044초 |

24건 완료, 정오답 변화 없음, 종료 코드 0·OOM 없음, 최고 RSS 2,517,264 KiB였다. CPU quota와 물리 코어 고정은 다르며, 스레드를 2개로 조정한 최적화 실험도 아니다. 각 조건 1회 실행으로 실행 시점·열·호스트 부하 차이를 통제하지 않았다. 원본은 `.local/picksy-dev/decider-q8/short-q4-cpu2-1.json`과 `.stderr`, 컨테이너는 `picksy-decider-q4-short-cpu2`다.

조회한 [Oracle 공식 Always Free 문서](https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier_topic-Always_Free_Resources.htm)는 A1 무료 한도를 월 1,500 OCPU 시간·9,000 GB 시간(2 OCPU·12 GB 상당)으로 안내한다. 실제 계정의 할당량·VM shape는 미확인이다. 과거의 4 OCPU·24 GB를 기본 전제로 삼지 않는다. 메모리 용량 증가만으로 추론 가속을 기대할 근거는 없으며, ARM 커널·메모리 대역폭·CPU 차이 때문에 Oracle이 더 빠르거나 느린지는 네이티브 ARM64 실행 전에는 미확인이다. 로컬 자원 감소 시 지연 증가가 관측됐으므로 무료 VM으로 옮기면 자동으로 빨라진다는 가정은 하지 않는다.

### 의미와 제약

- config의 `isolated_levels=true`를 보존했다. score는 각 수준을 독립적인 yes/no 적합성 질문으로 판단한 뒤 정규화한 확률의 기대값이다. 세 수준 score와 choice 묶음은 내부적으로 여러 행을 계산하므로 choice 하나의 지연으로 해석하지 않는다.
- 원본 온도는 choice 2.465·noul 0.05·score 2.833이며 수정하지 않았다. 기본 noul의 확률 보정 문제는 공식 카드에 명시돼 있다. Q8/긴 score 평가에는 noul이 없었고, 후속 Q4 짧은 평가에서는 5/6이었다.
- GGUF 파일명·카드는 v12지만 config의 `version`은 `2b-v11`로 남아 실제 응답도 `decider-2b-v11`이다. 이름을 임의 수정하지 않고 고정 저장소 리비전·파일 해시로 추적한다.
- 실제 로그는 모든 계층 CPU 배치와 Q8 모델 로딩을 보인다. CPU_REPACK 대신 CPU 버퍼를 사용한다는 안내가 있지만 지연의 주원인으로 확정하지 않는다.
- 공식 카드의 언어 표기는 영어다. 이번 한국어 합성 표본 통과는 게임·OMP 실제 작업의 수용 시험이 아니다. OMP·HTTP 서버는 연결하지 않았다.
- 가중치는 Apache-2.0 표시지만 공식 v12 카드가 비상업·연구용 학습 출처를 명시한다. 연구 실험과 제품 사용 허용 판단을 분리한다.

[실험 코드·리비전·해시·재현 절차](../../experiments/decider-q8/README.md), [공식 GGUF 카드](https://huggingface.co/Mapika/decider-2b-GGUF/blob/0df001b8fe4e5298c5dd45d875d3795fcdaea0ab/README.md), [가중치 카드·학습 출처](https://huggingface.co/Mapika/decider-2b).

## Kev-0.8B CPU 대조

[local-jev-bench](https://github.com/tak-bro/local-jev-bench)의 소형 판단 모델 후보를 바탕으로 공식 CPU 경로를 실제 실행했다. 해당 저장소의 M3 Max·Metal 속도는 Intel/Oracle CPU 예측에 사용하지 않았다. 실험 소스는 채택 후 삭제 승인 대상이며 아래 고정 리비전·로컬 결과를 근거로 보존한다.

- 소스 `2074cc44256200bb37540103fa4572901faa10f3`, Kev `v1.0` 어댑터 실제 SHA `bf75a6a8848ea6960ff2ed108d9ed44c2941174f`, 기반 모델 `dc7cdfe2ee4154fa7e30f5b51ca41bfa40174e68`. 다운로드 LFS 해시와 오프라인 캐시 snapshot을 확인했다.
- 이미지 `picksy-kev-08:cpu`, digest `sha256:2c522ac158391b1274cfd8a55006675061818f95b0e8edbd1d8ac0668319e1e4`. PyTorch 2.8.0+cpu·transformers 5.22.2·peft 0.21.2, CPU fp32·4스레드·원본 온도 2.3510958125672174.
- Intel Docker amd64, CPU 4개분·6 GiB·swap 추가 없음·네트워크 차단. 첫 추론 포함, 로딩 제외, 무워밍업·동시성 1. 공식 `Checkpoint.load → admit → model.probs → to_answers` 직접 호출이며 HTTP·OMP 종단 검증은 아니다.

| 평가 | Decider Q4 정답 | Kev-0.8B 정답 | Decider Q4 평균 | Kev 평균 |
|---|---:|---:|---:|---:|
| 기존 한국어 score | 17/18 | 17/18 | 아래 묶음 기준 | 아래 묶음 기준 |
| 기존 한국어 choice | 18/18 | 16/18 | 아래 묶음 기준 | 아래 묶음 기준 |
| 기존 한국어 score+choice 요청 | — | — | 8.689초 | 4.540초 |
| 짧은 게임 choice | 4/6 | 2/6 | 1.383초 | 1.257초 |
| 짧은 OMP 분류 choice | 5/6 | 5/6 | 1.226초 | 0.760초 |
| 짧은 OMP 조건 noul | 5/6 | 4/6 | 0.794초 | 0.784초 |
| 짧은 게임 score | 5/6 | 3/6 | 3.690초 | 1.113초 |

42개 요청이 모두 검증 완료됐고 컨테이너 두 개 모두 종료 코드 0·OOM 없음이다. 두 fixture 해시·입력·기대값·질문·선택지를 기존 Q4와 대조했다. score는 ±0.5, choice는 선택 ID 일치, noul은 0.5 초과/미만 기준이다. `original-1.json`의 choice `within_half_correct=17`은 확률 기대값 지표이며 선택 ID 정답 수가 아니다. 실제 choice 정답은 사례별 `correct.choice` 합계 16이고 `argmax_accuracy=16/18`과 일치한다. 기존 score gate는 통과했지만 새 업무 전체 수용을 뜻하지 않는다.

주요 실패는 체력·물약 규칙 적용, 문서만 변경하라는 요구에 코드도 바꾼 결과를 승인(`noul=0.8283`), 역순 정렬을 오름차순으로 승인(`0.7248`), 안전 구역 score `0.9135`다. README 오타 수정의 docs/fix 중첩은 기존 rubric 한계도 남아 있다. 안전 구역·현재 피격 등 짧은 게임 score는 3/6에 그쳤다.

최고 프로세스 RSS는 기존 18건 5,100,228 KiB, 짧은 24건 5,060,820 KiB로 Q4의 약 2.5백만 KiB보다 컸다. 로딩 시간은 각각 96.8초·61.4초이며 요청 지연에서 제외했다. 로그는 `causal_conv1d`·`flash-linear-attention` 미설치에 따른 reference PyTorch 경로를 명시한다. 이 경로의 실제 성능이며 최적화된 커널 성능으로 해석하지 않는다. 입력 절단·CPU 외 파라미터·dtype 변경은 없었다.

현재 선택은 Decider Q4 유지다. Kev는 일부 경로가 빨랐지만 한국어 짧은 게임 판단 품질과 메모리에서 불리했다. 공식 카드도 영어 전용이며 도구 호출 라우팅을 권장하지 않는다. 같은 정밀도의 모델 비교가 아니라 Q4 GGUF 대 fp32 PyTorch 실행 구성 비교이고, 각 1회 실행이므로 작은 시간 차이는 유의한 개선으로 단정하지 않는다. Jev API 재호출·파인튜닝·온도 변경·Oracle ARM 실행은 하지 않았다.

원본은 `.local/picksy-dev/kev-08/{original-1,short-1}.json`과 각각의 `.stderr`, 모델 출처는 `download-manifest.json`이다. [공식 모델 카드](https://huggingface.co/jaredpalmer/kev-0.8b/blob/bf75a6a8848ea6960ff2ed108d9ed44c2941174f/README.md), [고정 소스](https://github.com/jaredpalmer/kev/tree/2074cc44256200bb37540103fa4572901faa10f3).

## Jeff-0.8B legacy v1.2 CPU 대조

벤치마크 실행 스크립트는 소스 커밋을 고정하지만 가중치 다운로드 revision을 생략하고, [한국어 결과 header](https://github.com/tak-bro/local-jev-bench/blob/main/bench/runs/klue-ynat/jeff-0.8b.jsonl)의 `hub_revision`도 null이다. 따라서 동일 가중치 재현이 아닌 legacy v1.2 독립 실험을 선택했다. 현행 `jeff-base v1.4`와 어댑터는 평가하지 않았다.

당시 재현 조건: 모델 `jeff-legacy/Jeff-Qwen3.5-0.8B@d86f657f503245be5291e3c69e573bf48033aa50`, 공식 v1.2 소스 `bb9ae3b3ef18c3cf128bbc056cc2d0b388e5d115`, 이미지 digest `sha256:23277c792a12bf2a208cf8608df7343294dd475dc9490edc5b573b678fd9a4dd`. PyTorch 2.14.0+cpu·transformers 5.17.0, CPU fp32·4스레드·원본 온도 1.0845714325759162·state-first. 가중치와 설정의 SHA-256을 검사했고 CPU 배치·eval 모드·원본 온도를 확인했다. 실험 소스 삭제 승인과 원본 결과 보존을 구분한다. [고정 가중치](https://huggingface.co/jeff-legacy/Jeff-Qwen3.5-0.8B/tree/d86f657f503245be5291e3c69e573bf48033aa50).

기존 짧은 한국어 24개를 같은 질문·선택지 순서·정답으로 실행했다. Intel Docker amd64·CPU 4개분·6 GiB·네트워크 차단, 질문 하나씩·동시성 1·첫 추론 포함·로딩 제외·무워밍업. 공식 `load_decision_model → EvaluationRequest → server.predict` 경로로, HTTP·OMP 종단 지연은 아니다.

| 유형 | Decider Q4 정답 / 평균 | Kev-0.8B 정답 / 평균 | Jeff v1.2 정답 / 평균 | Jeff 중앙값 |
|---|---:|---:|---:|---:|
| 게임 choice | 4/6 · 1.383초 | 2/6 · 1.257초 | 2/6 · 4.742초 | 1.579초 |
| OMP 분류 choice | 5/6 · 1.226초 | 5/6 · 0.760초 | 5/6 · 0.992초 | 0.993초 |
| OMP 조건 noul | 5/6 · 0.794초 | 4/6 · 0.784초 | 5/6 · 1.601초 | 1.464초 |
| 게임 score | 5/6 · 3.690초 | 3/6 · 1.113초 | 6/6 · 2.790초 | 2.452초 |

Jeff 첫 요청 `g-c1`은 19.894초로 게임 choice 평균을 크게 올렸다. 같은 그룹 나머지 5개 평균은 1.711초다. 원인은 분리 측정하지 않았고 초회 비용을 임의로 빼서 주 비교 수치를 바꾸지 않았다. 각 조건 1회 실행이므로 호스트 부하·초기화 차이가 있으며 작은 시간 차이는 유의성 검증이 아니다.

24건 모두 응답·확률·선택 ID·score 기대값 검증을 통과하고 컨테이너는 종료 코드 0·OOM 없이 종료했다. 로딩 39.8초, 최고 RSS 5,585,048 KiB(약 5.33 GiB). 공식 CPU reference PyTorch 경로를 사용했고 `causal_conv1d`·`flash-linear-attention` 미설치 경고를 숨기지 않았다.

실패:
- 게임 choice는 6개 모두 attack을 골라 회복·후퇴 조건 4개를 틀렸다.
- OMP README 오타 수정은 docs 대신 fix를 선택했다. 기존 rubric 중첩 한계도 그대로다.
- 정상 오름차순 `[1,2,3]`을 요구 미충족(`noul=0.3986`)으로 판단했다.

게임 score 6/6은 긍정적이나, 안전 구역 `g-s1`의 0.4931은 허용폭 ±0.5에 가까워 안정성을 단정하지 않는다. 범용 후보를 Jeff로 교체할 근거는 부족하며 Decider Q4를 유지하는 것이 현재 권고다. Jeff의 score만 별도 후보로 볼 여지는 있지만 6건으로 라우팅 분리·운영 적용을 결정하지 않는다. 기존 18개 평가로 확대하거나 v1.4·ARM64·Jev API를 실행하지 않았다. 원본은 `.local/picksy-dev/jeff-08/short-1.json`, `.stderr`, `download-manifest.json`에 보존한다.

## 이전 Laya 고정 모델과 라이선스

- Laya 소스: `3a38cbe83f6ef556f3125fb0065fdc5a55d05792`, Apache-2.0.
- 모델: `convaiinnovations/laya-multilingual`, revision `1720e3e3357cfe1e281542e223f8273b0890ca34`, 모델 카드에 Apache-2.0 명시.
- 원본 `model.safetensors` SHA-256: `9d628fd971b700382ac6f65920a86f149777b2e748e0c955fb3b19695aa8f204`.
- 모델이 사용하는 `jhu-clsp/mmBERT-base` 카드는 MIT를 명시한다. 배포 시 해당 라이선스·고지와 의존성 라이선스를 보존·검토해야 한다. 이것은 전체 배포물의 법률 검토 완료를 뜻하지 않는다.
- 모델 카드와 설정은 기본 1024 토큰·head 256 토큰 및 미보정 temperature를 명시한다. 모델 카드는 score 위치 편향과 과신도 별도로 경고한다.
- 추가 비교한 `convaiinnovations/laya` 루트 english와 `convaiinnovations/laya-typed-decisions`는 약 421M ModernBERT-large 기반이며, 두 체크포인트와 기반 `answerdotai/ModernBERT-large` 카드 모두 Apache-2.0을 명시한다. typed-decisions는 특정 업무 흐름용이며 범용 다국어 모델이 아니다.
- 공식 Dockerfile과 CI에는 AMD64·ARM64 CPU 설치 경로가 있지만 실제 Oracle ARM 추론은 미검증이다.

## 이전 Laya 실행상의 주의

모델 디렉터리는 읽기 전용으로 마운트했다. Laya `_fix_tokenizer_config`가 `extra_special_tokens` 목록을 매핑으로 바꾸려다 읽기 전용 경고를 냈다. 이번 transformers 버전에서는 원본 그대로 로딩·추론이 성공했으나 다른 버전의 동작을 보장하지 않는다. 경고를 숨기거나 원본 파일을 덮어쓰지 않았다. 이 경고가 품질 실패의 원인이라고 단정할 근거는 없다.

로컬 경로로 로딩한 health 응답의 revision은 null이었다. 실제 리비전은 다운로드 시 고정한 SHA와 파일 해시로 추적한다. health 200만으로 준비 상태·정확한 모델을 확인했다고 간주하지 않는다.

## 남은 판단

- 단계 2 진행은 2026-11-13 승인됐다. 품질 위험을 명시한 인프라 검증이며 모든 판단 종류의 수용이나 출시 승인이 아니다. score를 임의로 제거하거나 다른 타입으로 몰래 대체하지 않는다.
- OMP 실제 종단 연동, 실제 코딩·게임 등 제품 대표 작업의 평가·보정, 장문, Oracle ARM CPU, Decider 두 개의 자원 사용은 미검증이다. 과거 Jev 비교와 추가 합성 score 평가는 제품 대표 작업의 수용 시험을 대체하지 않는다.

## 근거와 관련 결정

- [실험 코드·재현 조건](../../experiments/stage1/README.md), [단계 1 검증 기록](history/2026-11-12-laya-cpu-evaluation.md).
- [고정 모델 카드](https://huggingface.co/convaiinnovations/laya-multilingual/blob/1720e3e3357cfe1e281542e223f8273b0890ca34/README.md).
- [기반 encoder 모델 카드](https://huggingface.co/jhu-clsp/mmBERT-base).
- [고정 english 모델 카드](https://huggingface.co/convaiinnovations/laya/blob/85cc74ddae6109077ebe2853a9b3077fb6606a78/README.md), [고정 typed-decisions 모델 카드](https://huggingface.co/convaiinnovations/laya-typed-decisions/blob/e929ae5cf69bc34259cd2f95c9e91145b818b1f0/README.md), [ModernBERT-large 카드](https://huggingface.co/answerdotai/ModernBERT-large).
- [고정 공식 Dockerfile](https://github.com/NandhaKishorM/laya/blob/3a38cbe83f6ef556f3125fb0065fdc5a55d05792/Dockerfile), [공식 Docker CI](https://github.com/NandhaKishorM/laya/blob/3a38cbe83f6ef556f3125fb0065fdc5a55d05792/.github/workflows/docker.yml).
- [초기 설계 인계](history/2026-11-12-initial-design.md)는 당시 미검증 상태의 기록이다.
