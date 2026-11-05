# syntax=docker/dockerfile:1
# vy-serve som statisk musl-binär med referenssidan hello som paket.
#
#   docker build -t vy-serve .
#   docker run -p 8080:8080 -e VY_HOST_NAME=localhost:8080 vy-serve
#
# Signeringsnyckel: ge ett 32-byte frö som byggsecret `vy_key`
# (`docker build --secret id=vy_key,src=dev.key .`); saknas den skapas en
# tillfällig nyckel för bygget, och klienter pinnar då en ny nyckel per bygge.
# Fröet lämnar aldrig byggsteget; bara dev.key.pub följer med i bilden.

FROM rust:1.97-slim-bookworm AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add x86_64-unknown-linux-musl wasm32-unknown-unknown
WORKDIR /src
COPY . .
ENV CC_x86_64_unknown_linux_musl=musl-gcc
RUN cargo build --release --locked --target x86_64-unknown-linux-musl -p vy-serve -p vy-publish
RUN cd modules/hello && cargo build --release --locked --target wasm32-unknown-unknown
RUN --mount=type=secret,id=vy_key,required=false \
    set -eu; \
    bin=target/x86_64-unknown-linux-musl/release; \
    mkdir -p /srv /tmp/key; \
    if [ -s /run/secrets/vy_key ]; then cp /run/secrets/vy_key /tmp/key/dev.key; \
    else $bin/vy-publish keygen --out /tmp/key/dev.key; fi; \
    $bin/vy-publish pubkey --key /tmp/key/dev.key --out /srv/dev.key.pub; \
    $bin/vy-publish build \
        --manifest conformance/manifests/hello.json \
        --module modules/hello/target/wasm32-unknown-unknown/release/hello.wasm \
        --key /tmp/key/dev.key --out /srv/hello.vyp; \
    rm -rf /tmp/key

FROM scratch
COPY --from=build /src/target/x86_64-unknown-linux-musl/release/vy-serve /vy-serve
COPY --from=build /srv /srv
ENV VY_DIR=/srv VY_PUBLIC_KEY=/srv/dev.key.pub PORT=8080
EXPOSE 8080
ENTRYPOINT ["/vy-serve"]
