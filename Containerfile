FROM node:24-bookworm-slim AS web
WORKDIR /build
COPY package.json package-lock.json ./
COPY apps/interface/package.json apps/interface/package.json
COPY apps/desktop/package.json apps/desktop/package.json
RUN npm ci
COPY apps/interface apps/interface
RUN npm run web:build

FROM rust:1.97.1-bookworm AS rust
WORKDIR /build
ENV CARGO_BUILD_JOBS=2
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo build --locked --release --jobs 2 -p photo-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 photo-viewer \
    && useradd --uid 10001 --gid 10001 --home-dir /nonexistent --shell /usr/sbin/nologin photo-viewer \
    && install -d -o 10001 -g 10001 /var/lib/photo-viewer /var/cache/photo-viewer /app/web
COPY --from=rust /build/target/release/photo-server /usr/local/bin/photo-server
COPY --from=web /build/apps/interface/dist /app/web
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/photo-server"]
