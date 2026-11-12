"""고정 합성 상태의 score·choice 묶음을 로컬 GGUF 라이브러리로 평가한다."""

import argparse
import collections
import hashlib
import importlib.metadata
import inspect
import json
import math
import os
from pathlib import Path
import platform
import resource
import statistics
import sys
import time

REVISION = "0df001b8fe4e5298c5dd45d875d3795fcdaea0ab"
MEASUREMENT_ID = "98f82b99-597a-4d7c-949c-139b2e2c11cd"
GATE = {"within_half_accuracy": 0.8, "normalized_mae_max": 0.15,
        "per_level_accuracy_min": 2 / 3}
OPTIONS = {"n_gpu_layers": 0, "n_threads": 4, "n_ctx": 2048,
           "n_seq_max": 1, "verbose": True}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def number(value):
    require(type(value) in (int, float) and math.isfinite(value), "유한 수가 아닌 응답")
    return value


def memory():
    status = Path("/proc/self/status").read_text()
    return {line.split(":")[0]: int(line.split()[1]) for line in status.splitlines()
            if line.startswith(("VmRSS:", "VmHWM:"))}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def validate(kind, answer, expected):
    require(isinstance(answer, dict) and answer.get("type") == kind, "응답 타입 불일치")
    keys = ["0", "1", "2"] if kind == "score" else ["A", "B", "C"]
    probabilities = answer["probabilities"]
    require(isinstance(probabilities, dict) and set(probabilities) == set(keys), "확률 키 불일치")
    p = [number(probabilities[k]) for k in keys]
    require(all(0 <= x <= 1 for x in p), "확률 범위 오류")
    # 공식 format_answer는 확률을 소수 4자리, score를 소수 2자리로 반올림한다.
    require(abs(sum(p) - 1) <= 0.000151, "확률 합 오류")
    expectation = sum(i * x for i, x in enumerate(p))
    for key in ("confidence", "certainty", "x_p_max"):
        require(0 <= number(answer[key]) <= 1, "confidence 범위 오류")
    argmax = max(range(3), key=p.__getitem__)
    if kind == "score":
        value = number(answer["score"])
        require(0 <= value <= 2 and abs(value - expectation) <= 0.0052,
                "score와 반올림 확률 기대값 불일치")
        require(set(answer["legend"]) == set(keys), "score legend 불일치")
    else:
        require(answer["choice"] in keys, "choice A/B/C 매핑 오류")
        # 반올림 때문에 같은 확률로 보이는 선택지는 모두 최대값으로 허용한다.
        require(probabilities[answer["choice"]] == max(p), "choice 최대확률 불일치")
        argmax = keys.index(answer["choice"])
        value = expectation
    return {"semantic_expectation": value, "probability_expectation": expectation,
            "semantic_argmax": argmax, "within_half": abs(value - expected) <= 0.5}


def summarize(rows):
    return {"count": len(rows),
            "within_half_correct": sum(r["within_half"] for r in rows),
            "within_half_accuracy": statistics.mean(r["within_half"] for r in rows),
            "argmax_accuracy": statistics.mean(r["semantic_argmax"] == r["expected"] for r in rows),
            "normalized_mae": statistics.mean(abs(r["semantic_expectation"] - r["expected"]) for r in rows) / 2,
            "per_level_accuracy": {str(level): statistics.mean(r["within_half"] for r in rows if r["expected"] == level)
                                   for level in range(3)}}


def run(report, args):
    raw = Path("/experiment-stage1/score-cases.json").read_bytes()
    fixture = json.loads(raw)
    cases = fixture["cases"]
    require(len(cases) == len({c["id"] for c in cases}) == 18, "fixture 18쌍 필요")
    require(set(fixture["domains"]) == {"declared_priority", "urgency", "sentiment"}, "fixture 도메인 변경")
    counts = collections.Counter((c["domain"], c["expected"]) for c in cases)
    for domain in fixture["domains"]:
        for level in range(3):
            require(counts[domain, level] == 2, "fixture 수준 분포 변경")
    report["manifest_sha256"] = hashlib.sha256(raw).hexdigest()
    report["packages"] = {d.metadata["Name"]: d.version for d in importlib.metadata.distributions()}
    require(importlib.metadata.version("decider-ai") == "1.10.0", "decider 버전 불일치")
    require(importlib.metadata.version("llama-cpp-python") == "0.3.36", "llama 버전 불일치")
    model_dir = Path("/models/decider")
    model_path = model_dir / f"decider-2b-v12-{args.quantization}.gguf"
    config = json.loads((model_dir / "decider_config.json").read_text())
    report["model_config"] = config
    report["model_files_sha256"] = {str(p.relative_to(model_dir)): digest(p)
                                     for p in sorted(model_dir.rglob("*")) if p.is_file()}
    import torch
    from decider.infer import Decider
    import decider.infer
    import decider.systemone
    import decider.engine_gguf
    from decider.systemone import render_state
    torch.set_num_threads(4)
    report["installed_source_sha256"] = {m.__name__: digest(Path(inspect.getfile(m)))
                                          for m in (decider.infer, decider.systemone, decider.engine_gguf)}
    started = time.perf_counter()
    # temperature・isolated_levels는 원본 config에서 읽는다. 별도 보정/워밍업은 없다.
    model = Decider(str(model_path), device="cpu", gguf_options=OPTIONS)
    report["load_ms"] = (time.perf_counter() - started) * 1000
    report["effective_model"] = {"name": model.name, "temperature": model.T,
                                 "temperature_by_type": model.T_by_type,
                                 "isolated_levels": model.isolated_levels, "layout": model.layout,
                                 "engine": model.eng.cfg}
    report["memory_after_load_kib"] = memory()
    metrics = []
    for domain, spec in fixture["domains"].items():
        for language in ("ko", "en"):
            levels = spec["criteria"][language]
            require(len(levels) == 3, "fixture 수준 개수 변경")
            questions = {"score": {"type": "score", "instructions": spec["instructions"][language], "criteria": levels},
                         "choice": {"type": "choice", "instructions": spec["instructions"][language],
                                    "criteria": dict(zip(("A", "B", "C"), levels))}}
            batch = {"domain": domain, "language": language, "questions": questions, "cases": []}
            report["batches"].append(batch)
            batch_start = time.perf_counter()
            for case in cases:
                if case["domain"] != domain:
                    continue
                text = case["state"][language]
                require(isinstance(text, str) and bool(text.strip()), "빈 fixture 상태")
                state = text if args.condition == "original" else {"text": text, "measurement_id": MEASUREMENT_ID}
                row = {"id": case["id"], "expected": case["expected"], "state": state}
                batch["cases"].append(row)
                # 짧은 fixture만 허용해 기본 state 절단을 실제로 거치지 않음을 확인한다.
                tokens = len(model.m.tok.encode(render_state(state), add_special_tokens=False))
                row["state_tokens"] = tokens
                require(tokens < 2048, "state 토큰 한도 초과")
                started = time.perf_counter()
                try:
                    row["body"] = model.system_one(state, questions, max_state_tokens=2048)
                finally:
                    row["elapsed_ms"] = (time.perf_counter() - started) * 1000
                body = row["body"]
                require(body["model"] == model.name, "응답 모델 불일치")
                require(set(body["answers"]) == set(questions), "응답 질문 누락")
                row["metrics"] = {}
                for kind in questions:
                    metric = validate(kind, body["answers"][kind], case["expected"])
                    row["metrics"][kind] = metric
                    metrics.append({"language": language, "domain": domain, "condition": kind,
                                    "expected": case["expected"], **metric})
                row["memory_kib"] = memory()
            batch["elapsed_ms"] = (time.perf_counter() - batch_start) * 1000
    require(sum(len(b["cases"]) for b in report["batches"]) == 36, "36개 요청 미완료")
    report["summary"] = {f"{lang}/{kind}": summarize([r for r in metrics if r["language"] == lang and r["condition"] == kind])
                         for lang in ("ko", "en") for kind in ("score", "choice")}
    report["by_domain"] = {f"{lang}/{domain}/{kind}": summarize([r for r in metrics if r["language"] == lang and r["domain"] == domain and r["condition"] == kind])
                           for lang in ("ko", "en") for domain in fixture["domains"] for kind in ("score", "choice")}
    report["gate_passed"] = {}
    for language in ("ko", "en"):
        summary = report["summary"][language + "/score"]
        report["gate_passed"][language] = (summary["within_half_accuracy"] >= GATE["within_half_accuracy"]
                                            and summary["normalized_mae"] <= GATE["normalized_mae_max"]
                                            and min(summary["per_level_accuracy"].values()) >= GATE["per_level_accuracy_min"])
    report["batch_elapsed_ms_sum"] = sum(b["elapsed_ms"] for b in report["batches"])
    report["request_elapsed_ms_sum"] = sum(r["elapsed_ms"] for b in report["batches"] for r in b["cases"])
    report["completed"] = True
    return 0 if all(report["gate_passed"].values()) else 2


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--condition", required=True, choices=("original", "paired2"))
    parser.add_argument("--quantization", choices=("Q8_0", "Q4_K_M"), default="Q8_0")
    args = parser.parse_args()
    report = {"condition": args.condition, "model_revision": REVISION,
              "quantization": args.quantization,
              "measurement_id": MEASUREMENT_ID if args.condition == "paired2" else None,
              "measurement": "로컬 라이브러리 system_one 대기시간; HTTP/OMP 종단 측정 아님",
              "platform": platform.platform(), "machine": platform.machine(),
              "gguf_options": OPTIONS, "n_threads_batch": 4, "gate": GATE,
              "concurrency": 1, "questions_per_request": 2, "warmup": False,
              "completed": False, "batches": []}
    # 네이티브 stdout도 stderr로 보내 JSON 한 문서와 진단 로그를 분리한다.
    output_fd = os.dup(sys.stdout.fileno())
    sys.stdout.flush()
    os.dup2(sys.stderr.fileno(), sys.stdout.fileno())
    try:
        return run(report, args)
    except Exception as error:
        report["failure"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        usage = resource.getrusage(resource.RUSAGE_SELF)
        report["process_cpu_seconds"] = usage.ru_utime + usage.ru_stime
        report["process_peak_rss_kib"] = usage.ru_maxrss
        # 잘못된 응답도 원문 body에 보존하고 엄격 검사는 위에서 실패시킨다.
        with os.fdopen(output_fd, "w") as output:
            print(json.dumps(report, ensure_ascii=False, indent=2), file=output, flush=True)


if __name__ == "__main__":
    sys.exit(main())
