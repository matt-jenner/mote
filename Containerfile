FROM node:24-bookworm-slim AS web
WORKDIR /build
COPY package.json package-lock.json ./
COPY apps/interface/package.json apps/interface/package.json
COPY apps/desktop/package.json apps/desktop/package.json
RUN npm ci
COPY docs/brand/fonts docs/brand/fonts
COPY docs/brand/svg docs/brand/svg
COPY apps/interface apps/interface
RUN npm run web:build

FROM rust:1.97.1-bookworm AS native-build
ARG MOTE_HEIC=enabled
WORKDIR /build
COPY THIRD_PARTY_NOTICES.md THIRD_PARTY_NOTICES.md
COPY packaging/licenses packaging/licenses
COPY packaging/heic packaging/heic
COPY scripts/build-lifecycle.sh scripts/build-lifecycle.sh
RUN case "$MOTE_HEIC" in enabled|disabled) ;; *) exit 2 ;; esac \
    && if [ "$MOTE_HEIC" = enabled ]; then \
      apt-get update && apt-get install -y --no-install-recommends cmake ninja-build pkg-config \
      && rm -rf /var/lib/apt/lists/* \
      && packaging/heic/build-unix.sh --platform linux --arch "$(uname -m)"; \
    fi

FROM native-build AS rust
ARG MOTE_HEIC=enabled
ENV CARGO_BUILD_JOBS=2
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN if [ "$MOTE_HEIC" = enabled ]; then \
      export PKG_CONFIG_PATH="/build/build/heic-native/linux-$(uname -m)/lib/pkgconfig"; \
      cargo build --locked --release --jobs 2 -p photo-server; \
    else \
      cargo build --locked --release --jobs 2 -p photo-server --no-default-features --features mote-defaults; \
    fi

FROM rust AS runtime-copy
ARG MOTE_HEIC=enabled
RUN install -Dm0755 target/release/photo-server /runtime/usr/local/bin/photo-server \
    && if [ "$MOTE_HEIC" = enabled ]; then \
      install -d /runtime/usr/local/lib /runtime/etc/ld.so.conf.d; \
      cp -a build/heic-native/linux-*/lib/libheif.so.* build/heic-native/linux-*/lib/libde265.so.* /runtime/usr/local/lib/; \
      install -Dm0644 THIRD_PARTY_NOTICES.md /runtime/usr/share/licenses/mote/THIRD_PARTY_NOTICES.md; \
      install -Dm0644 packaging/licenses/LGPL-3.0-or-later.txt /runtime/usr/share/licenses/mote/LGPL-3.0-or-later.txt; \
      install -Dm0644 packaging/licenses/libheif.md /runtime/usr/share/licenses/mote/libheif.md; \
      install -Dm0644 packaging/licenses/libde265.md /runtime/usr/share/licenses/mote/libde265.md; \
      install -Dm0644 packaging/heic/README.md /runtime/usr/share/licenses/mote/HEIC-REBUILD.md; \
      install -Dm0644 packaging/heic/decode-only.cmake /runtime/usr/share/licenses/mote/decode-only.cmake; \
      printf '%s\n' /usr/local/lib > /runtime/etc/ld.so.conf.d/mote.conf; \
    fi
RUN LD_LIBRARY_PATH=/runtime/usr/local/lib sh packaging/heic/verify-linux-runtime.sh \
    /runtime /runtime/usr/local/bin/photo-server "$MOTE_HEIC"

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libstdc++6 libgcc-s1 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 photo-viewer \
    && useradd --uid 10001 --gid 10001 --home-dir /nonexistent --shell /usr/sbin/nologin photo-viewer \
    && install -d -o 10001 -g 10001 /var/lib/photo-viewer /var/cache/photo-viewer /app/web
COPY --from=runtime-copy /runtime/ /
COPY --from=web /build/apps/interface/dist /app/web
RUN ldconfig
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/photo-server"]
