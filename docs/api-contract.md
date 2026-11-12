# Picksy TypeSafe API 계약과 검증 경계

[PRD](prd.md) · [추론 Wiki](wiki/inference.md) · [단계별 계획](plans/implementation-plan.md)

## 범위와 근거 수준

현재 채택 엔진은 Decider-2B Q4_K_M이다. 아래에서 현재 확인한 라이브러리 계약과 이전 Laya HTTP 계약을 분리한다. Decider 라이브러리 호출 성공을 HTTP 래퍼·제품 Gateway 구현 완료로 해석하지 않는다. OMP → Gateway → Decider 종단 검증은 단계 4 대상이다.

- 설치된 OMP 버전: 18.19.0. 설치물은 단일 바이너리이며 아래 코드는 같은 버전의 공식 `v18.19.0` 태그에서 확인했다. 바이너리와 태그의 바이트 단위 동일성을 검증한 것은 아니다.
- 이전 비교 엔진: Laya 0.4.31, 소스 `3a38cbe83f6ef556f3125fb0065fdc5a55d05792`.
- 이전 비교 모델: `convaiinnovations/laya-multilingual`, revision `1720e3e3357cfe1e281542e223f8273b0890ca34`. 현재 채택 모델이 아니다.
- 첫 제품 범위는 텍스트와 JSON state에 대한 `choice`, `noul`, `score`다. OMP가 지원하는 이미지·assets·box까지 현재 엔진·HTTP 래퍼가 지원한다고 가정하지 않는다.

## 현재 Decider 라이브러리 계약

- 채택: `Mapika/decider-2b-GGUF@0df001b8fe4e5298c5dd45d875d3795fcdaea0ab`의 `decider-2b-v12-Q4_K_M.gguf`. SHA-256 `e6d3a7534a3abc3a723f8263dc694b86f533214a5bf761474113c66d44018340`.
- 실제 실행: `decider-ai==1.10.0`, `llama-cpp-python==0.3.36`, CPU 직접 `system_one` 호출. 문자열/JSON state, 질문 ID별 `choice`·`noul`·`score`를 평가했다. HTTP 상태·인증·취소·동시성 계약은 이 라이브러리 결과만으로 확정하지 않는다.
- `choice`는 선택 ID와 선택지 확률, `noul`은 true 확률이다. `score`는 0-based 수준 확률의 기대값을 소수 2자리로 반올림하고 확률은 소수 4자리로 반환한다. 원본 `isolated_levels=true`에서 각 수준을 독립 yes/no로 계산한 뒤 분포를 정규화한다. 이를 단일 choice 추론이나 Laya와 동일한 확률 계산으로 취급하지 않는다.
- `confidence`를 정답 확률 또는 Jev/Laya와 동일한 보정값이라고 설명하지 않는다. 누락된 usage·토큰·비용을 합성하지 않으며 필수 필드·finite·범위·선택지·반올림 허용오차를 검증한다.
- config의 `version=2b-v11` 때문에 원본 응답 이름은 `decider-2b-v11`이다. v12 파일명·revision·해시로 실행 모델을 식별하고 이름을 몰래 수정하지 않는다. 외부 모델 ID·모델 카드와 내부 응답명 매핑은 HTTP 구현에서 명시적으로 정해야 한다.
- OMP의 `/v1/models`, `/v1/systemone`, Bearer 인증 요구와 질문 ID 보존은 유지한다. 공개 목록·허용 목록·HTTP 오류 스키마·제한값은 Gateway/래퍼 구현에서 확정하며 아래 Laya 기본값을 그대로 이식하지 않는다.
- 단계 2의 HTTP 래퍼·풀은 승인된 구현/검증 범위이지 이 문서에서 완료를 보장하는 계약이 아니다. 이미지·assets·box 지원도 확인하지 않았다.

근거: [실행 코드·공식 wheel 소스 해시](../experiments/decider-q8/README.md), [고정 GGUF 카드](https://huggingface.co/Mapika/decider-2b-GGUF/blob/0df001b8fe4e5298c5dd45d875d3795fcdaea0ab/README.md), [실측과 한계](wiki/inference.md), [채택 결정](wiki/history/2026-11-13-decider-q4-adoption.md).

## 이전 Laya HTTP 계약 — 역사적 비교 근거

이하 1~5절의 모델 카드·상한·confidence·오류는 **Laya 0.4.31 당시 관측과 설계안**이다. 현재 Decider HTTP 계약이 아니다. OMP 소스 확인 사실과 인증·의미 보존 원칙은 재사용하지만 Laya 전용 동작을 Decider에 가정하지 않는다.

## 1. 공개 경로와 인증

| 경로 | Gateway 책임 | Laya 0.4.31의 실제 코드 |
|---|---|---|
| `GET /v1/models` | 인증 후 지원하는 고정 모델 목록 제공 | 해당 경로가 없다. upstream 목록을 그대로 프록시할 수 없다. |
| `POST /v1/systemone` | 인증·입력·모델·제한 검사 후 내부 풀 호출 | 같은 경로를 제공한다. |

OMP의 TypeSafe wire는 base URL 뒤에 `/v1/systemone`을 붙인다. 예를 들어 base URL은 `https://picksy.example` 형태이지 `https://picksy.example/v1`이 아니다. 예시 호스트는 실제 배포 주소가 아니다.

공개 API는 `Authorization: Bearer <클라이언트 키>`를 요구한다. 외부 키와 Laya 내부 인증 자격 증명은 별도 책임이다. Gateway는 클라이언트가 보낸 헤더 전체를 내부 서버에 무조건 전달하지 않는다.

## 2. 모델 목록

OMP의 `parseTypeSafeModelCards`가 요구하는 형태:

```json
{
  "models": [
    {
      "name": "laya-multilingual",
      "description": "Laya 다국어 텍스트 판단 모델",
      "release_date": ""
    }
  ]
}
```

`name`은 trim 후 비어 있지 않은 문자열, `description`·`release_date`는 문자열이어야 한다. 이 버전의 OMP parser는 빈 날짜를 허용한다. 모델 수정일을 릴리스 날짜로 꾸미지 않는다. 위 항목은 Gateway가 제공할 초기 모델 카드이며 실제 구현·응답이 아니다.

- 공개 `laya-multilingual`은 내부 `model: "multilingual"`과 위 고정 체크포인트에 명시적으로 연결한다.
- 알 수 없는 모델과 `jev-*`는 거절한다. 이름만 Jev로 바꾸거나 묵시적으로 Laya로 대체하지 않는다.
- Laya의 `LAYA_MODELS`는 preload 목록이지 요청 허용 목록이 아니다. Gateway가 별도 허용 목록을 유지해야 한다.
- OMP 내장 `typesafe` provider에는 Jev seed가 남을 수 있다. 향후 연동에서는 별도 Picksy provider와 명시적 모델 ID를 우선하며 실제 사용자 설정 변경은 별도 요청에 따른다.

## 3. 판단 요청

```json
{
  "model": "laya-multilingual",
  "state": "중복 결제 금액을 환불해 주세요.",
  "questions": {
    "department": {
      "type": "choice",
      "instructions": "담당 부서를 선택하세요.",
      "criteria": {
        "billing": "결제와 환불",
        "technical": "오류와 장애"
      }
    }
  }
}
```

| 필드 | OMP 계약 및 Gateway 적용 규칙 |
|---|---|
| `state` | 문자열, JSON 객체 또는 배열. 누락·null은 거절한다. |
| `model` | 공개 허용 모델을 요구하며 내부 checkpoint 이름으로 명시적으로 매핑한다. |
| `questions` | question ID를 키로 하는 객체. ID를 변경하지 않는다. |
| `choice` | `instructions` 문자열과 `criteria: {선택지: 설명 또는 null}`. 선택지 키와 순서를 보존한다. |
| `noul` | `instructions` 문자열, 선택적인 `criteria: {"true": 설명, "false": 설명}`. true/false 의미를 바꾸지 않는다. |
| `score` | `instructions` 문자열과 낮은 수준부터 높은 수준 순서의 설명 배열. 순서와 0-based 위치를 보존한다. |

빈 질문, 지원하지 않는 타입·모델, 잘못된 필드 타입은 추론 전에 거절한다. 이미지·assets·box 등 지원하지 않는 기능은 조용히 무시하지 않고 거절한다. Laya의 `max_len`, `head_max_len`, `task`, `lang`, hooks 같은 확장 필드를 그대로 외부에 개방하지 않는다.

Upstream 기본 HTTP 상한은 질문 64개, choice 옵션 100개, score 수준 32개, 전체 옵션 512개, state 50,000자, body 2 MiB다. 이는 검증된 품질 범위나 안전한 동시 처리량이 아니다. Gateway 제한값은 이를 넘기지 않도록 단계 3·4에서 자원 예산과 함께 확정한다. 다국어 모델은 기본 1024 토큰, head 256 토큰이며 50,000자 허용이 전체 내용 판독을 보장하지 않는다. 잘림 정보는 숨기지 않으며 제품의 장문 허용 범위는 별도 검증한다.

## 4. 응답 의미와 변환

최상위 핵심 필드는 `model`, `answers`, `usage`다. 질문 ID마다 요청한 타입의 답이 있어야 한다.

| 답 타입 | 값의 의미 | 보존할 내용 |
|---|---|---|
| `choice` | 선택지 중 argmax, 선택지별 확률 | 선택지 키와 확률을 그대로 유지 |
| `noul` | `noul`은 true일 확률, 0..1 | boolean으로 변환하지 않음 |
| `score` | `sum(index * probability)`, 0..(수준 수-1)의 기대값 | 임의의 0..1 정규화·반올림 분류를 하지 않음 |
| `confidence` | Laya choice/score는 `1 - H(p)/log(k)` | 정답 확률이나 Jev 동일 confidence로 설명하지 않음 |
| `answer_confidence` | Laya의 별도 값. 이번 체크포인트는 미보정 상태 | 필드 이름만으로 보정된 확률이라고 보장하지 않음 |
| `usage` | upstream의 입력·출력 토큰 및 추가 사용량 정보 | 누락값·비용·토큰을 임의로 만들어 채우지 않음 |

Laya `Agent`의 `model`은 `laya-rl-agent` 같은 일반 식별자일 수 있다. 체크포인트 식별은 명시적 요청 매핑과 `routing.model`·고정 revision으로 확인해야 한다. Gateway는 내부 경로를 외부 모델명으로 노출하거나 잘못 실행된 모델을 요청 모델명으로 덮어써 숨기지 않는다.

OMP의 공식 decoder는 최상위 추가 필드를 무시하고, 요청한 answer는 객체와 `type`만 검사한 뒤 추가 필드를 포함해 전달한다. 따라서 `routing`, `legend`, `answer_confidence` 때문에 strict 모드를 켤 필요는 없다. 반대로 OMP가 값의 범위까지 검증해 줄 것이라고 믿어서는 안 된다. Gateway는 필수 답·타입·finite 값·선택지·확률 범위와 실제 라우팅 모델을 확인해야 한다.

`LAYA_JEV_STRICT=1`은 추가 필드를 제거할 뿐 confidence의 계산 의미를 Jev로 바꾸지 않는다. `x_jev_confidence`로 기존 confidence를 몰래 대체하지 않는다. 초기 규칙은 원래 의미 보존이다.

## 5. 오류와 재시도 경계

Laya 코드에서 확인한 상태는 인증 401, JSON·필수 입력 400, 크기 초과 413, 질문·모델 경로·토큰 한도 오류 422, body 수신 시간 초과 408, admission 초과 503, 추론 실패 500이다. 알 수 없는 단순 문자열 모델은 자동 라우팅되지만 경로나 미등록 Hub ID는 422다. Gateway는 이 느슨한 모델 해석을 사용하지 않는다.

Gateway 구현 규칙:

- 외부 키 실패는 401, 제한 초과는 429, 과부하·미준비는 503, Gateway 추론 대기 기한 초과는 504로 구분한다.
- 입력 오류는 400/413/422로 구분하고 공개된 오류 코드와 안전한 메시지를 반환한다.
- 내부 인증 실패·잘못된 upstream 응답·추론 실패는 upstream 상세 본문을 노출하지 않는 502 계열 오류로 취급한다. 내부 인증 실패를 클라이언트 키 오류처럼 401로 전달하지 않는다.
- 구체적인 오류 코드 이름·JSON 스키마·시간 제한은 단계 3·4에서 구현 전 확정한다. 실패를 빈 answers나 정상 판단으로 대체하지 않는다.

OMP `NativeJudge` 기본은 시도당 10초, 최대 3회다. 네트워크 오류와 HTTP 408/429/5xx를 재시도하며 외부 취소 시 중단한다. backoff는 기본 500ms부터 시작하고 `Retry-After`를 고려하되 최대 5초로 제한한다. 상위 판단 라우터의 기한은 별도로 더 짧을 수 있다. 따라서 Gateway 자체에 추가 추론 재시도를 기본 도입하지 않으며 취소·중복 실행·사용량 정책을 후속 구현에서 검증한다.

## 공식 근거

- [OMP v18.19.0 wire serializer/decoder](https://github.com/can1357/oh-my-pi/blob/v18.19.0/packages/ai/src/judgment/systemone.ts): `typesafeWire`, `decodeSystemOne`.
- [OMP 질문·답 타입](https://github.com/can1357/oh-my-pi/blob/v18.19.0/packages/ai/src/judgment/types.ts).
- [OMP NativeJudge](https://github.com/can1357/oh-my-pi/blob/v18.19.0/packages/ai/src/judgment/native.ts): `#judgeOnce`, `#attempt`, `isAnswerOf`.
- [OMP 모델 목록 parser](https://github.com/can1357/oh-my-pi/blob/v18.19.0/packages/catalog/src/discovery/typesafe.ts).
- [OMP TypeSafe seed](https://github.com/can1357/oh-my-pi/blob/v18.19.0/packages/catalog/src/compat/rules/providers/typesafe.kdl).
- [고정 Laya HTTP 서버](https://github.com/NandhaKishorM/laya/blob/3a38cbe83f6ef556f3125fb0065fdc5a55d05792/laya/serve.py): `create_app`, `_resolve_model`, `_project_jev_strict`.
- [고정 Laya 답 생성](https://github.com/NandhaKishorM/laya/blob/3a38cbe83f6ef556f3125fb0065fdc5a55d05792/laya/agent.py): `_decode_answers`.
- [고정 Laya 입력·confidence 처리](https://github.com/NandhaKishorM/laya/blob/3a38cbe83f6ef556f3125fb0065fdc5a55d05792/laya/common.py): `render_options`, `confidence_from_probs`.
