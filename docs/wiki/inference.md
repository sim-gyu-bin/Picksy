# 추론과 클라이언트 연동

[Wiki](wiki.md) · [API 계약](../api-contract.md) · [채택 결정](history/2026-11-13-d1-q4-adoption.md)

## 현재 지식

**d1-2B Q4_K_M 채택 합의 · 로컬 단일 서버 실측 완료 · 단계 4 진행 승인·미구현.** Rust Gateway 기반은 단계 3까지 완료했지만 실제 추론·OMP 종단 연결과 Rust+d1+Caddy 재통합은 아직 검증하지 않았다.

### 고정 모델과 런타임

- 모델: `LiquidAI/d1-2B-GGUF@4a181094d1b6addfaa3a0956f78e7ab2aa406a0c`.
- 파일: `d1-2B-Q4_K_M.gguf`, 1,099,629,888 bytes, SHA-256 `adca5789ad30ae6027e8e936749bb9f19673097492ab26fa1c5b69643ca50b4b`.
- 공식 llama.cpp 이미지: `ghcr.io/ggml-org/llama.cpp@sha256:2103d1f4f2e2e223ad80587dcb39fc15a9f912f29ae1fa9fc1193fe8c1aea82f`.
- 실제 버전: `v0.9.0-dev`, build 12316, commit `5d4c8eeb2`. Intel Docker amd64 CPU quota·스레드 4, 메모리 6 GiB, context/batch/ubatch 2048, slot 1, GPU layers 0.
- 공식 `POST /v1/systemone`의 choice·noul·score를 실제 HTTP로 평가했다. score는 누적확률이 0.5에 처음 도달하는 중앙값 등급이고 confidence는 최대 확률이다. 확률 가중 평균·정답 확률로 바꾸지 않는다.
- 라이선스는 [LFM Open License 1.0](https://huggingface.co/LiquidAI/d1-2B-GGUF/blob/4a181094d1b6addfaa3a0956f78e7ab2aa406a0c/LICENSE)이다. 공식 안내상 연매출 $10M 미만은 상업 이용 무료, 이상은 별도 계약이 필요하다. Apache 라이선스가 아니며 배포 시 실제 조건·고지를 검토해야 한다.

### 현재 성능 관측

고정 한국어 합성 24건, 한 요청당 질문 하나, 동시성 1·순차 실행·첫 요청 포함·서버 준비 시간 제외다.

| 유형 | 정답 / 전체 | 평균 지연 |
|---|---:|---:|
| 게임 choice | 6/6 | 833.0ms |
| OMP choice | 5/6 | 718.1ms |
| OMP noul | 6/6 | 613.3ms |
| 게임 score | 6/6 | 928.3ms |
| 전체 | 23/24 | 773.2ms |

24건 순차 합계 18.56초, 최대 프로세스 RSS 1,901,772 KiB(약 1.81 GiB), 서버 준비 약 8.88초다. 모든 요청 HTTP 200·출력 토큰 0·계약 오류 0, 종료 코드 0·OOM 없음·시험 키 로그 미노출을 기존 실행에서 확인했다. 컨테이너 전체 메모리·병렬 처리량·SLA를 뜻하지 않는다.

## 제약과 확인 필요 사항

- 유일한 오답 `o-c3`은 README 설치 설명 오타 수정을 docs 대신 fix로 분류한 경우다. rubric 중첩 한계도 남으며 사후에 정답이나 입력을 바꾸지 않았다.
- choice는 선택 ID 일치, noul은 0.5 초과/미만(동률 오답), score는 사전 기대 등급 ±0.5로 평가했다. score 응답 자체를 기대값으로 변환한 것이 아니다.
- 각 조건 1회인 소규모 합성 평가다. 실제 한국어 코딩·게임 판단, 장문, 반복 변동, 부하, Oracle ARM64, d1 복제본 두 개, OMP 종단은 미검증이다. 직접 검사할 수 있는 수치·정렬 조건의 정확성을 모델에 보장하지 않는다.
- 응답 model의 내부 파일 경로를 외부에 노출하지 않도록 공개 모델 ID를 명시적으로 매핑해야 한다. 제품 목록·상한·오류·취소·동시성 계약과 OMP 최종 연결은 단계 4에서 확정한다. 누락 usage·비용을 꾸미지 않는다.

## 근거와 이력

- [공통 fixture](../../experiments/fixtures/short-cases.json), SHA-256 `3de61796fb6a012e519b031acc4d15a3a7b2b1c94ffc5d925ac63aa32fd6d4d0`.
- [현재 실행기](../../experiments/d1/evaluate.py) · [실행 절차](../../experiments/d1/README.md).
- 보존 결과: `.local/picksy-dev/d1-q4/run-a145621e1d/report.json`과 같은 폴더의 `server.log`.
- [고정 서버 구현](https://github.com/ggml-org/llama.cpp/blob/5d4c8eeb2/tools/server/server-decision.cpp): `format_answer`, `get_decision_confidence`.
- [이전 후보·양자화 비교, 해시·실패·라이선스·계약 이력](history/2026-11-13-inference-comparison.md) · [이전 API 계약](history/2026-11-13-api-contract-history.md) · [이전 풀 실험](history/2026-11-13-pool-validation.md).
