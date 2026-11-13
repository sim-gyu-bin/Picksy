# Picksy TypeSafe API 계약과 검증 경계

[PRD](prd.md) · [추론 Wiki](wiki/inference.md) · [단계별 계획](plans/implementation-plan.md)

## 범위와 근거 수준

현재 채택은 d1-2B Q4_K_M과 공식 llama.cpp HTTP 서버다. 고정 모델·이미지·환경은 [추론 Wiki](wiki/inference.md)를 따른다. 단일 서버의 합성 입력 실측과 Rust Gateway·OMP 종단 연동은 다르다. **단계 3 완료, 단계 4 사용자 진행 승인·미구현**이며 운영 배포 승인이 아니다.

## 현재 d1 HTTP 관측과 의미

- 공식 `POST /v1/systemone`에서 텍스트 state와 질문 ID별 choice·noul·score를 실제 호출했다. 제품 범위는 텍스트와 JSON state이며 이미지·assets·box 지원은 가정하지 않는다.
- choice는 선택 ID와 선택지 확률을 반환한다. noul은 true 확률이며 boolean으로 변환하지 않는다.
- score는 낮음부터 높음 순서의 0-based 등급에서 **누적확률이 0.5에 처음 도달하는 중앙값 등급**이다. 확률 가중 평균이나 임의의 0..1 점수로 변환하지 않는다.
- confidence는 최대 확률이다. 정답 확률·다른 엔진의 보정값과 같다고 가정하지 않는다. 질문 ID·선택지·등급 순서·확률 의미를 보존한다.
- 24건의 관측은 모두 HTTP 200·출력 토큰 0이었다. 누락 usage·토큰·비용을 합성하지 않는다. 원본 model에는 컨테이너 파일 경로가 들어가므로 제품 공개 ID와 내부 고정 모델 매핑을 명시해야 한다.
- 단일 서버 실험에서 인증 없는 모델 목록 요청 401과 `/health` 200을 확인했다. 이는 제품 Gateway 목록 스키마·인증 경계·추론 준비 계약을 검증한 것이 아니다.

근거: [고정 서버 소스](https://github.com/ggml-org/llama.cpp/blob/5d4c8eeb2/tools/server/server-decision.cpp)의 `format_answer`, `get_decision_confidence`, [현재 실행기](../experiments/d1/evaluate.py), [실행 절차](../experiments/d1/README.md).

## 단계 3 Gateway 기반 계약

현재 추론 경로를 흉내 내지 않고 인증·DB 기반만 구현한다. 실제 실행 검증 상태는 [작업계획서](plans/implementation-plan.md), 운영자 명령은 [로컬 개발](wiki/local-development.md)을 따른다.

| 경로 | 현재 동작 |
|---|---|
| `GET /health/live` | 인증 없는 생존 확인, 200. 모델·DB 준비를 보장하지 않는다. |
| `GET /health/ready` | 인증 없는 DB 필수 테이블 조회, 정상 200·실패 503. 추론 준비는 아직 포함하지 않는다. |
| `GET /v1/auth/check` | Bearer 인증·공유 키별 한도·사용량 접수/완료 후 본문 없는 204. 추론을 수행하지 않는다. |
| `/v1/models`, `/v1/systemone` | 아직 등록하지 않아 404. 단계 4에서 실제 추론 계약을 연결한다. |

- 키 형식은 `pk_`와 OS 난수 32바이트의 소문자 16진수 64자리다. 키 ID는 별도 UUID이며 원문 키는 발급 CLI에서만 제공한다. 공개 키 관리 HTTP 경로는 없다.
- 인증 누락·형식 오류·중복 Authorization·알 수 없는 키·폐기 키는 동일하게 401과 `{"error":"unauthorized"}`를 반환한다. Bearer scheme은 대소문자를 구분하지 않는다.
- 키별 기본 한도는 서비스 전체 60회/분이며 발급 시 지정한다. 초과는 429와 `{"error":"rate_limited"}`, 다음 고정 분 구간까지의 초 단위 `Retry-After`를 반환한다. 두 Gateway가 같은 SQLite를 사용해야 같은 한도를 공유한다.
- DB 인증·제한·접수·완료 기록 실패는 503과 `{"error":"unavailable"}`다. DB 장애를 성공이나 빈 사용량으로 바꾸지 않는다. 사용량 완료는 응답 전 DB 기록이며 클라이언트 수신 확인은 아니다.
- 응답에는 서버가 만든 `x-request-id`와 `Cache-Control: no-store`를 붙인다. 클라이언트가 보낸 요청 ID는 신뢰하지 않는다. 로그에는 요청 ID·상태·소요 시간만 남기고 URI·헤더·본문을 기록하지 않는다.
- 폐기는 이후 DB 검사를 거치는 요청에 반영한다. 이미 접수된 요청은 완료할 수 있다. 초기 생존/준비 경로의 공개 상태에는 내부 주소·DB 내용·모델 정보가 없다.



## 단계 4 구현·검증 경계

- `/v1/models`, `/v1/systemone`, Bearer 인증과 실제 OMP 연결이 목표다. 설치 OMP 18.19.0의 같은 버전 공식 태그 조사와 설치 바이너리의 실제 종단 검증을 구별한다.
- 후속 OMP 18.19.0 소스 조사에서 `NativeJudge` 런타임이 d1 중앙값 score를 수용하는 경로를 확인했다. 다만 `ScoreAnswer` 문서의 기대값 설명과 의미가 다르므로 호환성을 의미 동일성으로 설명하지 않는다. 중앙값·최대 확률을 무보정으로 유지하며 실제 설치 바이너리의 종단 실행은 아직 미실행이다.
- 공개 모델 ID `d1-2b-q4`는 권장안이며 단계 4에서 확정한다. 설치 바이너리의 `NativeJudge`·`fetchTypeSafeModels`를 통한 검증 경로는 확보했지만 모델 목록·판단 실행 성공으로 기록하지 않는다.
- 공개 모델 카드·허용 목록·요청/응답 최종 스키마·오류 매핑·상한·동시성·대기·타임아웃·취소는 구현 전에 확정한다. 자료가 없는 세부 계약을 단일 서버 관측으로 채우지 않는다.
- 외부 요청은 Gateway 인증을 거친다. 내부 서버에 외부 Authorization·임의 라우팅 헤더를 그대로 전달하거나 내부 경로를 노출하지 않는다.
- 필수 답·타입·finite·확률 범위·선택지·실제 모델을 검증하며 실패를 빈 answers나 정상 판단으로 대체하지 않는다. 추가 자동 추론 재시도는 기본 도입하지 않는다.
- [이전 라이브러리·HTTP 계약 및 OMP 조사 원본](wiki/history/2026-11-13-api-contract-history.md)은 역사적 근거다. 당시 상한·오류·모델 카드·confidence를 현재 서버 계약에 이식하지 않는다.
