FROM picksy-stage1-laya:0.4.31

USER root
# 기존 CPU torch/transformers를 교체하지 않는다. 충돌은 resolver 오류로 드러낸다.
RUN apt-get update && apt-get install -y --no-install-recommends build-essential cmake ninja-build
COPY experiments/decider-q8/constraints.txt /experiment-decider/constraints.txt
ENV CMAKE_ARGS="-DGGML_CUDA=OFF -DGGML_METAL=OFF -DGGML_NATIVE=OFF" \
    CMAKE_BUILD_PARALLEL_LEVEL=4
RUN python -m pip install --no-cache-dir -c /experiment-decider/constraints.txt \
    'decider-ai==1.10.0' 'llama-cpp-python==0.3.36' \
    && python -m pip check
COPY experiments/decider-q8/evaluate.py /experiment-decider/evaluate.py
ENV HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 HF_HUB_DISABLE_TELEMETRY=1 \
    OMP_NUM_THREADS=4 MKL_NUM_THREADS=4 OPENBLAS_NUM_THREADS=4 \
    TOKENIZERS_PARALLELISM=false PYTHONUNBUFFERED=1
WORKDIR /experiment-decider
ENTRYPOINT ["python", "/experiment-decider/evaluate.py"]
