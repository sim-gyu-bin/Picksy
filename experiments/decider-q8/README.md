# Decider-2B Q4_K_M 주력 평가와 기존 Q8 비교

2026-11-13 사용자가 Q4_K_M을 채택하고 단계 2 진행을 승인했다. 디렉터리 이름 `decider-q8`은 기존 결과·명령의 경로를 유지하기 위한 것으로 현재 주력 양자화는 Q4다. 기존 runner 기본값 Q8_0과 아래 Q8 재현 예시는 비교용이며 Q4는 `--quantization Q4_K_M`을 명시한다.

Linux amd64 직접 CPU 추론 결과를 보존한다. 짧은 한국어 24건의 게임 choice 4/6·OMP choice 5/6·noul 5/6·score 5/6과 지연 한계는 미해결이다. 이 디렉터리의 실험은 HTTP 래퍼·OMP 종단 연동이나 운영 검증이 아니다. [채택 결정](../../docs/wiki/history/2026-11-13-decider-q4-adoption.md).

## 설치와 실행

기존 로컬 `picksy-stage1-laya:0.4.31` 이미지(Python 3.11 이상, torch `2.14.0+cpu`, transformers `5.22.2`)를 기반으로 한 역사적 빌드 경로다. stage1 Dockerfile은 정리 대상이므로 깨끗한 checkout에서 이 기반 이미지를 새로 만들 수 있다고 보장하지 않는다. 보존된 Decider 이미지 digest로 직접 CPU 실험을 재사용하며 단계 2용 독립 이미지 구성과 구분한다. 공식 PyPI의 `decider-ai==1.10.0`, `llama-cpp-python==0.3.36`을 사용했고 정상 resolver 설치와 전체 버전 기록을 유지했다. 의존성 충돌·`pip check` 실패를 우회하지 않는다. 설치 네트워크와 차단된 추론 런타임을 구분한다.

저장소 루트에서 실행한다. `MODEL_DIR`는 고정 revision `0df001b8fe4e5298c5dd45d875d3795fcdaea0ab`에서 받은 `decider-2b-v12-Q8_0.gguf`, `decider_config.json`, tokenizer 파일들이 있는 절대 경로다. `RESULT_DIR`는 미리 준비한 결과 디렉터리이며 기존 결과를 덮어쓰지 않는 이름을 사용한다.

```sh
docker build -f experiments/decider-q8/Dockerfile -t picksy-decider-q8:1.10.0 .
docker run --name picksy-decider-original --platform linux/amd64 --network none --cpus 4 --memory 6g --memory-swap 6g \
  -v "$MODEL_DIR:/models/decider:ro" \
  -v "$PWD/experiments/stage1:/experiment-stage1:ro" \
  picksy-decider-q8:1.10.0 --condition original \
  > "$RESULT_DIR/decider-original.json" 2> "$RESULT_DIR/decider-original.stderr"
docker run --name picksy-decider-paired2 --platform linux/amd64 --network none --cpus 4 --memory 6g --memory-swap 6g \
  -v "$MODEL_DIR:/models/decider:ro" \
  -v "$PWD/experiments/stage1:/experiment-stage1:ro" \
  picksy-decider-q8:1.10.0 --condition paired2 \
  > "$RESULT_DIR/decider-paired2.json" 2> "$RESULT_DIR/decider-paired2.stderr"
```

두 실행은 각각 새 프로세스에서 36개 상태·72개 판단을 수행한다. 종료 코드 0은 두 언어 score gate 통과, 2는 정상 완료 후 품질 미달이다. 예외는 부분 보고서를 stdout에 남기고 traceback을 stderr에 출력하며 실패 종료한다. 컨테이너 강제종료/OOM/native abort는 Python finally를 실행하지 못하므로 Docker 종료 상태와 stderr도 확인해야 한다. 컨테이너·파일 자동 삭제는 하지 않는다.

## 조건과 의미

- `/experiment-stage1/score-cases.json`의 기존 18개 한국어·영어 쌍만 사용한다. 도메인→언어→사례 순서와 score·choice 두 질문 묶음은 기존 `evaluate-latency.py`와 같다.
- `original`: 원문 문자열 상태. `paired2`: `{text: 원문, measurement_id: "98f82b99-597a-4d7c-949c-139b2e2c11cd"}` 상태. 두 조건 결과를 합쳐 품질을 주장하지 않는다.
- `n_gpu_layers=0`, `n_ctx=2048`, `n_threads=4`, `n_seq_max=1`. 공식 `GGUFEngine`은 `n_threads_batch` 인자를 받지 않으며 `n_threads`를 `cp.n_threads`와 `cp.n_threads_batch` 모두에 설정하므로 두 값이 4다. 기본 independent 동작과 config의 `isolated_levels`·temperature를 보존한다. isolated score는 각 수준의 yes/no 적합 확률을 정규화한 분포이고, 여러 내부 forward가 발생할 수 있다.
- `system_one` score는 0-based 확률 기대값을 소수 2자리로 반올림한다. 확률은 소수 4자리다. 합 허용오차 0.000151, 기대값 일치 허용오차 0.0052는 이 반올림만 고려한다. 타입·finite·범위·확률 키·합·score 기대값·choice 최대확률을 검사한다. choice A/B/C는 수준 0/1/2이며 기대값과 실제 선택 정확도를 별도 기록한다. confidence는 정답 확률로 해석하지 않는다.
- 기존 gate 그대로 언어별 score ±0.5 정확도 ≥0.8, 정규화 MAE ≤0.15, 수준별 정확도 ≥2/3이다. 정규화 MAE는 폭 2로 나눈다. score 평가는 반환 score, choice 기대값은 반환 확률을 사용하며 임의 재보정하지 않는다.
- 준비·파일 해시 시간을 제외하고 첫 추론을 포함한다. 별도 워밍업은 없다. 개별 `elapsed_ms`는 **라이브러리 system_one 호출 대기시간**이며 HTTP 대기시간이 아니다. `batch_elapsed_ms_sum`은 검증·메모리 기록 등 루프 부대 작업을 포함한 여섯 배치 벽시계 합이다. Laya HTTP·Jev 원격 helper와 순수 모델 속도 비교로 해석하지 않는다.
- 원본 응답·입력·개별 시간·RSS/HWM·CPU 시간·전체 패키지·모델 파일 SHA-256·실행 소스 SHA-256을 보존한다. config의 오래된 `version=2b-v11` 때문에 응답 이름은 `decider-2b-v11`일 수 있다. 이를 v12로 고치지 않으며 v12 파일명/고정 다운로드 revision/파일 해시와 원본 메타데이터를 구분한다. runner의 revision 문자열 자체는 다운로드 출처 검증이 아니므로 다운로드 manifest와 대조해야 한다.

### Q4 대조 실행

같은 GGUF 리비전의 `decider-2b-v12-Q4_K_M.gguf`만 추가하고 `--quantization Q4_K_M`을 지정한다. 기본값은 기존 Q8_0이다. fixture·판정 기준·온도·isolated score·CPU 제한은 바꾸지 않는다. Q8 이미지에 들어 있는 이전 runner 대신 갱신된 이 디렉터리를 `/runner:ro`로 마운트해 같은 런타임을 재사용한다.

```sh
docker run --name picksy-decider-q4-original --platform linux/amd64 \
  --network none --cpus 4 --memory 6g --memory-swap 6g \
  --entrypoint python \
  -v "$MODEL_DIR:/models/decider:ro" \
  -v "$PWD/experiments/stage1:/experiment-stage1:ro" \
  -v "$PWD/experiments/decider-q8:/runner:ro" \
  sha256:c6632d172bb586d377bfebd7625348f1daf981818f325338c125f92aeab9f41f \
  /runner/evaluate.py --condition original --quantization Q4_K_M
```

paired2는 새 컨테이너 이름과 `--condition paired2`로 순차 실행하고 별도 보고서에 저장한다. 라이브러리·모델 파일 해시는 결과에 남긴다. 이전 Q8 결과를 덮어쓰거나 품질 통과를 위해 입력을 바꾸지 않는다.

실제 Q4 실행 완료: 두 조건 모두 36개 요청, 종료 코드 0·OOM 없음. 다운로드 근거는 로컬 `download-q4-manifest.json`, 결과는 `decider-q4-original-1.json`, `decider-q4-paired2-1.json`과 대응 `.stderr`다. Q4 파일은 1,274,396,192 bytes, SHA-256 `e6d3a7534a3abc3a723f8263dc694b86f533214a5bf761474113c66d44018340`. Q8과 공유하는 fixture·패키지·모델 config·실행 옵션을 대조했다. [품질·시간 비교](../../docs/wiki/inference.md#q4-대조-결과)가 현재 결과 원본이다.

### 짧은 질문 평가

`short-cases.json`은 게임 행동·변경 분류·조건 충족·대응 필요성의 한국어 합성 24건이다. 실제 로그나 OMP 호출을 재생한 것은 아니다. 같은 이미지·마운트·자원 제한에서 `/runner/evaluate-short.py`를 실행하면 질문 하나씩 순차 처리한다. 정답은 서버 입력에 넣지 않는다. choice 라벨 일치, noul 0.5 초과/미만(동률 오답), score ±0.5로 평가한다. 결과는 `short-q4-1.json`과 대응 stderr에 보존했다. 종료 코드 0은 실행 완료이지 제품 품질 통과가 아니다. [현재 결과·실패·기준 모호성](../../docs/wiki/inference.md#짧은-게임omp-모사-질문)을 함께 읽는다.

## 실행 근거

- 이미지 manifest: `sha256:c6632d172bb586d377bfebd7625348f1daf981818f325338c125f92aeab9f41f`, config: `sha256:550e61d0668681e924400423f2f015c75e5f1f11aab7fced6134340bc1d2b245`.
- Q8 파일: 2,012,012,064 bytes, SHA-256 `b89b9bed14dc889075de93136e9a9e3cd859c26b008bfb4bb0211ca0d7357c27`. 다운로드 manifest와 실행 보고서의 모든 모델 파일 해시를 대조했다.
- 로컬 보존 경로: `.local/picksy-dev/decider-q8/`. 모델은 `model/`, 다운로드 근거는 `download-manifest.json`, 원문 결과는 `decider-original-1.json`과 대응 `.stderr`다.
- 원문 상태의 한국어 score 17/18·choice 18/18, 영어 두 유형 모두 18/18. 두 언어 모두 기존 score gate를 통과했다. 한국어 명시 우선순위 높은 사례 한 건은 score 1.42로 ±0.5 기준에 미달했다.
- 원문 두 질문 묶음의 라이브러리 호출 평균 13,762ms·중앙값 12,721ms, 로딩 약 5.5초, 프로세스 최고 RSS 2,654,376 KiB. 해당 Intel Docker CPU 조건의 결과로 ARM 성능을 보장하지 않는다. 최신 실제 비교·한계는 [추론 Wiki](../../docs/wiki/inference.md)에 둔다.
- 원본 config는 `isolated_levels=true`여서 score의 각 수준을 별도 yes/no 적합성 질문으로 계산한다. 일반적인 choice 하나의 실행 시간으로 해석하지 않는다. 로그에는 CPU_REPACK 대신 CPU 버퍼를 사용한다는 안내가 있으며 이것이 지연의 주원인인지는 검증하지 않았다.
- v12 가중치 카드에 비상업 학습 데이터 출처가 명시돼 있다. Apache-2.0 표시만으로 제품 사용의 법적 검토가 끝난 것으로 보지 않는다.
- `paired2` 보고서는 `decider-paired2-1.json`과 대응 `.stderr`다. 두 조건 모두 종료 코드 0·OOM 없음으로 종료했다. 현재 품질·속도 비교의 원본은 [추론 Wiki의 Decider 절](../../docs/wiki/inference.md#decider-2b-q8-대체-후보)이다.
- 병목 계측: [profile.py](profile.py)를 같은 이미지의 Python entrypoint로 실행한다. 모델 마운트와 fixture 마운트는 동일하게 두고 이 디렉터리를 `/profile:ro`로 연결해 `/profile/profile.py`를 실행했다. 대표 한 건에 원문/paired2 × 묶음/choice 단독 4개 호출을 수행한다. 가중치·온도·계산 인자는 바꾸지 않고 네이티브 호출 구간과 CPU 제한 통계만 기록한다. 결과 `profile-1.json`·`.stderr`, 컨테이너 `picksy-decider-profile`. 실제 라이브러리 CPU 기능·설치 소스는 별도 `picksy-decider-diagnostics`에서 조회했다.

## 공식 근거

- [PyPI decider-ai 1.10.0](https://pypi.org/pypi/decider-ai/1.10.0/json): Python ≥3.11, transformers ≥5, GGUF extra llama-cpp-python ≥0.3.35,<0.4.
- [PyPI llama-cpp-python](https://pypi.org/pypi/llama-cpp-python/json): 확인 당시 0.3.36, yanked 아님.
- [infer.py](https://raw.githubusercontent.com/Mapika/decider/main/decider/infer.py), [systemone.py](https://raw.githubusercontent.com/Mapika/decider/main/decider/systemone.py), [engine_gguf.py](https://raw.githubusercontent.com/Mapika/decider/main/decider/engine_gguf.py): 공식 PyPI 1.10.0 wheel과 조회한 main 세 파일이 바이트 단위로 같음을 확인했다. 설치·추론 검증을 뜻하지 않는다.
- 위 wheel 소스 SHA-256: infer `a58e670c56f42855af94b8041b3bbd806f168d6b8aecf5c24776f069256aaa88`, systemone `884e4b0bae36f5e09dbd689b4390e94320cbbd77f7b8d675e50490b1d7c61d34`, engine_gguf `9db9ac5f4147d6236f5071edf08dcbe578df21079fe1cd10853a027aa5d7982a`.
