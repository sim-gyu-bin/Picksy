# 단계 1 Laya CPU 실험 기록과 공통 fixture

[작업계획서](../../docs/plans/implementation-plan.md) · [추론 Wiki](../../docs/wiki/inference.md) · [실행 이력](../../docs/wiki/history/2026-11-12-laya-cpu-evaluation.md)

2026-11-13 [Decider Q4 채택](../../docs/wiki/history/2026-11-13-decider-q4-adoption.md)으로 Laya 실험 소스는 정리 대상이다. 이 README와 [score-cases.json](score-cases.json)은 비교 근거·Decider 공통 입력으로 보존한다. 기존 Dockerfile·평가 runner를 현재 checkout에서 실행하는 안내는 제공하지 않는다. 아래는 당시 실제 실행 조건이며 HTTP Gateway·OMP 종단·운영 검증이 아니다.

## 고정 입력

- Laya 0.4.31: [소스 리비전](https://github.com/NandhaKishorM/laya/tree/3a38cbe83f6ef556f3125fb0065fdc5a55d05792).
- 모델: [convaiinnovations/laya-multilingual 리비전](https://huggingface.co/convaiinnovations/laya-multilingual/tree/1720e3e3357cfe1e281542e223f8273b0890ca34).
- 원본 소스: `.local/picksy-dev/stage1/laya-source/`. 모델: `.local/picksy-dev/stage1/model-multilingual/`.
- 공통 fixture는 명시 우선순위·상황별 긴급도·감정 각 수준 두 개씩의 한국어·영어 대응 18쌍이다. 모델 입력·정답·기준을 결과에 맞춰 변경하지 않았다.

## 실행 경계

당시 Linux amd64 CPU 4개·메모리 6 GiB·swap 추가 없음, `--network none`, 호스트 포트 미게시였다. 서버와 HTTP 클라이언트는 같은 컨테이너 loopback을 사용했다. 서버 키는 프로세스에서 임의 생성했고 사용자 인증·유료 API 설정을 재사용하지 않았다. 원본 모델을 읽기 전용으로 마운트하고 `HF_HUB_OFFLINE=1`을 사용했다.

공식 서버를 수정하지 않았으며 `LAYA_EXTRA_MODELS`로 고정 경로를 등록했다. `LAYA_MODELS`는 preload 목록이지 허용 목록이 아니었다. 이것은 이전 Laya 계약이며 Decider HTTP 계약으로 이식하지 않는다.

## 실행 전에 정한 판정 기준

최초 입력은 문의 choice 12개·환불 noul 4개·한국어 긴급도 score 3개였다. 탐색 기준은 choice 10/12 이상, noul 3/4 이상, score 2/3 이상이다. noul은 0.5 기준, score는 0..2 기대값이 정답 위치 ±0.5 이내인지 평가했다. 실제 결과는 11/12·4/4·1/3으로 score 미달이었다. 품질 실패 종료 코드 2를 정상 통과로 바꾸지 않았다.

## score 추가 대조 실험

공통 fixture의 36개 상태 각각에 정상 score·동일 의미 choice·역순 score·역순 choice를 실행하고 한국어 18개에 영어 지시문 score를 추가해 162개 요청을 수행했다. 역순 score는 원래 좌표로 `2 - score`를 계산하는 진단이며 운영 해결책이 아니다.

언어별 정상 score 탐색 기준은 정답 ±0.5 정확도 80% 이상, 정규화 MAE(오차/2) 0.15 이하, 각 수준 정확도 2/3 이상이었다. 이는 제품 출시 기준이 아니다. 다국어 모델의 두 언어 모두 정상 score 기준 미달이었다.

원본은 `.local/picksy-dev/stage1/evaluation-1.json`, `score-evaluation-1.json`에 있다.

## 공식 체크포인트 비교

같은 fixture·162개 요청·판정 기준·고정 이미지로 다음 체크포인트를 각각 순차 평가했다.

| 이름 | 공식 고정 모델 | revision |
|---|---|---|
| english | [convaiinnovations/laya](https://huggingface.co/convaiinnovations/laya/tree/85cc74ddae6109077ebe2853a9b3077fb6606a78) 루트 | `85cc74ddae6109077ebe2853a9b3077fb6606a78` |
| typed-decisions | [convaiinnovations/laya-typed-decisions](https://huggingface.co/convaiinnovations/laya-typed-decisions/tree/e929ae5cf69bc34259cd2f95c9e91145b818b1f0) | `e929ae5cf69bc34259cd2f95c9e91145b818b1f0` |

토큰 예산은 1024/head 256으로 고정했다. english 기본 512/192와 다르므로 기본 설정 성능을 뜻하지 않는다. 두 모델은 영어 ModernBERT-large 기반이며 한국어는 지원 밖 진단이다. 원본 temperature는 변경하지 않았지만 런타임이 `choice:11+` temperature 0.10058을 하한 0.5로 제한했다. 세 선택지 평가는 해당 항목을 사용하지 않았다.

english의 영어 score만 기준을 통과했고 한국어는 모든 후보에서 미달했다. 원본 모델은 `.local/picksy-dev/stage1/model-english/`, `model-typed-decisions/`, 결과는 `score-english-1.json`, `score-typed-decisions-1.json`이다. 정리 승인에 로컬 모델·결과 삭제는 포함되지 않는다.

## Jev와 동일 질문 묶음 지연 비교

공통 fixture 36개 상태 각각에 score·choice를 묶어 언어·도메인별 여섯 상태를 순차 실행했다. 준비 시간은 제외하고 첫 추론은 포함했다. Laya 로컬 HTTP와 당시 Jev 원격 OMP helper(`concurrency=1`, `retries=0`)의 사용자 대기시간 비교이지 순수 모델 속도 시험이 아니다.

Jev 재호출에서 캐시 재사용 의심 결과가 있어 양쪽에 동일한 `{"text": 원문, "measurement_id": UUID}` 상태를 전달한 별도 측정을 보존했다. 원문 조건과 섞지 않고 서버 캐시 제거를 보장하지 않는다. 현재 문서는 새 유료 호출을 승인하지 않는다. 정확한 당시 명령·이미지 digest·보고서 경로와 한계는 [실행 이력](../../docs/wiki/history/2026-11-12-laya-cpu-evaluation.md), 비교 수치는 [추론 Wiki](../../docs/wiki/inference.md)에 보존한다.

## 보존 자원

`.local/picksy-dev/stage1/` 원본 소스·모델·합성 결과와 기존 Docker 이미지는 유지한다. 과거 컨테이너 정리 이력과 이번 소스 정리 승인은 별개이며 추가 컨테이너·볼륨 삭제는 실행하지 않는다. Decider 직접 CPU 재현은 [보존한 Decider 실험](../decider-q8/README.md)을 따른다.
