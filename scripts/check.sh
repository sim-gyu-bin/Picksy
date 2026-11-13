#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# 로컬과 CI가 같은 검사 범위를 사용한다. 데이터 삭제나 운영 연결은 하지 않는다.
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
