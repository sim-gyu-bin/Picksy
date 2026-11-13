# Decider Q4 채택과 단계 2 승인

[결정 인덱스](history.md) · 현재 지식: [추론](../inference.md), [아키텍처](../architecture.md) · [작업계획서](../../plans/implementation-plan.md)

- 기록일: 2026-11-13
- 결정일: 2026-11-13
- 상태: 후속 결정으로 대체됨. 당시 합의는 단계 1 모델 선택 완료, 단계 2 진행 승인이었다.
- 출처: 사용자의 Decider-2B Q4_K_M 최종 채택·실험 소스 정리·후속 진행 요청. 아래 실측은 기존 실험 기록 재사용이며 이번 문서 편집에서 재실행하지 않았다.
- 후속 결정: [d1 Q4 채택과 단계 4 진행 승인](2026-11-13-d1-q4-adoption.md). 아래 모델 선택·미구현·보존 문구는 당시 판단이며 현재 채택·정리 범위는 후속 기록을 따른다.

## 배경과 결정

[초기 Laya 추천](2026-11-12-initial-design.md)과 [CPU 비교 이력](2026-11-12-laya-cpu-evaluation.md)을 거쳐 Decider-2B Q4_K_M을 채택한다. Gateway·추론 서버 독립 풀, 공개 Gateway 인증, 비공개 Caddy 내부 리스너, 운영 데이터 분리 원칙은 유지한다.

- 모델: `Mapika/decider-2b-GGUF@0df001b8fe4e5298c5dd45d875d3795fcdaea0ab`.
- 파일: `decider-2b-v12-Q4_K_M.gguf`, 1,274,396,192 bytes.
- SHA-256: `e6d3a7534a3abc3a723f8263dc694b86f533214a5bf761474113c66d44018340`.
- 런타임: `decider-ai==1.10.0`, `llama-cpp-python==0.3.36`, 직접 CPU 추론.
- config `version=2b-v11`과 응답 `decider-2b-v11`은 원본 그대로 보존한다. v12 파일명과 불일치를 숨기지 않고 revision·해시로 식별한다.

## 대안과 선택 이유

기존 동일 fixture의 Q4는 Q8 대비 정답 개수를 유지하면서 요청 대기가 약 37~38% 감소했다. 이는 전반적인 품질 무손실을 보장하지 않는다. Laya의 한국어 score 한계, Kev·Jeff의 짧은 게임 choice 실패와 더 큰 메모리 관측은 [비교 이력](2026-11-13-inference-comparison.md)에 보존한다. 일부 유형에서 다른 후보가 더 빠르거나 정확했으므로 Q4가 모든 과제에서 우월하다는 판단은 아니다. 최종 채택 근거는 이러한 한계를 확인한 뒤의 사용자 선택이다.

## 결과·단점·검증 상태

- 짧은 한국어 합성 24건: 게임 choice 4/6, OMP choice 5/6, noul 5/6, score 5/6. 규칙 적용·정렬 판단 실패와 rubric 중첩이 남는다.
- 지연·품질 위험을 유지한 채 단계 2 인프라 검증을 승인했다. HTTP 래퍼·독립 풀·OMP 종단·ARM64·제품 출시 검증 완료나 운영 배포 승인이 아니다. 단계 2 체크박스는 실제 검증 전 미완료다.
- 원본 `isolated_levels`·온도·확률 의미를 유지하며 실패를 숨기기 위한 score 제외·confidence 재보정·가짜 응답은 도입하지 않는다.
- 영어 표기 모델의 한국어 소규모 결과와 실제 제품 수용을 구분한다. 가중치 카드의 학습 출처·제품 사용 법률 검토도 해결됐다고 보지 않는다.

## 승인된 소스 정리와 보존

삭제 승인 범위는 `experiments/jeff-08/` 전체, `experiments/kev-08/` 전체, stage1의 `Dockerfile`, `evaluate.py`, `evaluate-score.py`, `evaluate-latency.py`다. 삭제 실행 여부는 실제 변경으로 판단한다. `experiments/stage1/score-cases.json`·README와 Decider 소스는 보존한다. `experiments/decider-q8/` 경로는 유지하되 Q4가 주력이고 Q8은 비교 이력이다.

`.local/picksy-dev/stage1/`, `.local/picksy-dev/decider-q8/`, `.local/picksy-dev/kev-08/`, `.local/picksy-dev/jeff-08/`의 모델·결과·다운로드 manifest 및 Docker 이미지·컨테이너·볼륨은 이번 삭제 승인에 포함되지 않는다. Wiki의 역사적 실패·비교 기록도 보존한다.

## 근거

- [당시 실험과 패키지 소스 해시 보존](2026-11-13-inference-comparison.md). 원래 경로: `experiments/decider-q8/README.md`.
- [고정 GGUF 모델 카드](https://huggingface.co/Mapika/decider-2b-GGUF/blob/0df001b8fe4e5298c5dd45d875d3795fcdaea0ab/README.md).
- 로컬 `.local/picksy-dev/decider-q8/download-q4-manifest.json`, `decider-q4-original-1.json`, `decider-q4-paired2-1.json`, `short-q4-1.json`과 대응 stderr. Git에 원본 결과를 추가하지 않는다.
