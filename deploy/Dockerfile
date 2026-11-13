# syntax=docker/dockerfile:1

FROM node:22.21.1-bookworm-slim AS frontend
WORKDIR /build/admin
COPY admin/package.json admin/package-lock.json ./
RUN npm ci
COPY admin/index.html admin/tsconfig.json admin/tailwind.config.cjs admin/LICENSE.shadcn ./
COPY admin/scripts ./scripts
COPY admin/src ./src
RUN npm run typecheck && npm run build

FROM rust:1.95.0-bookworm AS server
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --locked --release --bin picksy

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && groupadd --gid 10001 picksy \
    && useradd --uid 10001 --gid picksy --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin picksy \
    && install -d -o picksy -g picksy -m 0700 /var/lib/picksy/data
WORKDIR /app
COPY --from=server /build/target/release/picksy /usr/local/bin/picksy
COPY --from=frontend /build/admin/dist/index.html /app/admin/index.html
COPY --from=frontend /build/admin/dist/assets/app.js /app/admin/assets/app.js
COPY --from=frontend /build/admin/dist/assets/app.css /app/admin/assets/app.css
USER 10001:10001
EXPOSE 18081
# named volume은 /var/lib/picksy에 연결한다. 빈 볼륨은 쓰기 가능한 data 하위 경로를 복사한다.
# migrate/serve를 운영자가 분리 실행한다. 기본 명령은 도움말이며 자동 Migration은 없다.
ENTRYPOINT ["picksy", "--db", "/var/lib/picksy/data/gateway.sqlite3"]
CMD ["--help"]
