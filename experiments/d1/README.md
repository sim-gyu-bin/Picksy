# d1-2B Q4 CPU 평가

선택한 모델을 [한국어 합성 24건](../fixtures/short-cases.json)으로 순차 평가한다. 과거 후보 비교와 평가 계약 수정 경위는 [Wiki History](../../docs/wiki/history/2026-11-13-inference-comparison.md)에 보존한다.

## 고정 실행 조건

- 모델: `LiquidAI/d1-2B-GGUF@4a181094d1b6addfaa3a0956f78e7ab2aa406a0c`, `d1-2B-Q4_K_M.gguf`, 1,099,629,888바이트.
- SHA-256: `adca5789ad30ae6027e8e936749bb9f19673097492ab26fa1c5b69643ca50b4b`.
- 이미지: `ghcr.io/ggml-org/llama.cpp@sha256:2103d1f4f2e2e223ad80587dcb39fc15a9f912f29ae1fa9fc1193fe8c1aea82f`.
- 실제 버전: `0.9.0-dev`, build 12316, commit `5d4c8eeb2`, Linux x86_64.
- Intel Mac Docker amd64, CPU quota 4·추론/배치 스레드 4, 메모리 6 GiB·추가 swap 없음. context/batch/ubatch 2048, slot 1, GPU layers 0. 텍스트만 평가하므로 멀티모달 projector는 로딩하지 않는다.
- GGUF 해시 검사 후 읽기 전용 마운트. 모델의 확률·confidence·출력을 보정하지 않는다.
- loopback 임의 포트와 런타임 생성 Bearer 키를 사용한다. 키는 파일·보고서·로그에 기록하지 않는다. 인증 없는 모델 목록 요청 401, `/health` 200을 확인한다. 제품 Gateway 인증의 대체 경로가 아닌 독립된 로컬 합성 실험이다.
- 입력 SHA-256: `3de61796fb6a012e519b031acc4d15a3a7b2b1c94ffc5d925ac63aa32fd6d4d0`.

## 실행과 보존

모델 파일은 `.local/picksy-dev/d1-q4/`에 미리 고정 리비전으로 내려받는다. 평가 실행기는 파일을 다운로드하거나 덮어쓰지 않는다.

```sh
python3 -B experiments/d1/evaluate.py --image ghcr.io/ggml-org/llama.cpp@sha256:2103d1f4f2e2e223ad80587dcb39fc15a9f912f29ae1fa9fc1193fe8c1aea82f
```

기존 결과는 `.local/picksy-dev/d1-q4/run-a145621e1d/report.json`, 로그는 같은 폴더의 `server.log`에 있다. 다운로드 manifest는 상위 디렉터리에 있다. 보고서는 고정 합성 사례만 포함한다. 실행기는 종료 후 컨테이너·모델·보고서를 삭제하지 않는다.

모델 재호출 없이 저장 응답만 검사할 수 있다. 결과는 원본과 다른 `report-d1-contract.json`에 배타적으로 생성하며 기존 파일을 덮어쓰지 않는다.

```sh
python3 -B experiments/d1/evaluate.py --replay .local/picksy-dev/d1-q4/run-a145621e1d/report.json
```

## 평가 계약과 한계

- [고정 llama.cpp 소스](https://github.com/ggml-org/llama.cpp/blob/5d4c8eeb2/tools/server/server-decision.cpp)의 lfm3-d1 score는 누적확률이 0.5에 처음 도달하는 중앙값 등급이며, confidence는 최대 확률이다. 가중 평균으로 바꾸지 않는다.
- 확률 범위·합·키, score 중앙값·legend를 검사한다. 정답은 choice ID 일치, noul 엄격한 0.5 경계, score 기대값 ±0.5다.
- 기존 실행은 23/24 정답, HTTP 200 24건, 계약 오류 0이었다. 평균 HTTP 왕복 773.2ms, 순차 24건 합계 18.56초였다. 병렬 묶음 시간이 아니다.
- 준비 후 첫 요청을 포함하며 별도 평가 워밍업·추론 재시도는 없다. 로딩 시간은 요청 시간에서 제외한다.
- 소규모 합성 단일 실행은 실서비스 정확도·Jev 동급·Oracle ARM 지연·복제본 처리량을 보장하지 않는다.
- 원본 응답 `model`은 컨테이너 모델 경로다. Gateway 공개 모델 ID와의 매핑은 제품 계약에 명시해야 한다.
- [LFM Open License v1.0](https://huggingface.co/LiquidAI/d1-2B-GGUF/blob/4a181094d1b6addfaa3a0956f78e7ab2aa406a0c/LICENSE)의 상업 이용·배포 조건을 별도로 따른다.
