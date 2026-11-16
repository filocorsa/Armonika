# syntax=docker/dockerfile:1

FROM rust:1.99-slim-bookworm AS build
WORKDIR /src
# The workspace needs every member present for a locked build.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY bench/testdata ./bench/testdata
COPY bench/harness ./bench/harness
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p armonika-server \
    && cp target/release/armonika /armonika

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --user-group --no-create-home --shell /usr/sbin/nologin armonika \
    && mkdir /data \
    && chown armonika:armonika /data
COPY --from=build /armonika /usr/local/bin/armonika

ENV ARMONIKA_DATA_DIR=/data \
    ARMONIKA_SERVER__BIND=0.0.0.0:4747
USER armonika
WORKDIR /data
VOLUME /data
EXPOSE 4747
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["armonika", "healthcheck"]
ENTRYPOINT ["armonika"]
CMD ["serve"]
