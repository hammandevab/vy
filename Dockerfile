# syntax=docker/dockerfile:1
# vy-serve som statisk musl-binär med alla referenssidor som signerade paket
# och en förhandsvisning per sida (tools/publish_site.sh, samma steg lokalt).
#
#   docker build -t vy-serve .
#   docker run -p 8080:8080 -e VY_HOST_NAME=localhost:8080 vy-serve
#
# Signeringsnyckel: byggargumentet VY_KEY är utgivarnyckelns 32-byte frö som
# 64 hextecken (`od -An -v -tx1 dev.key | tr -d ' \n'` ur filen som
# `vy-publish keygen --out dev.key` skriver). Railway skickar tjänstens
# variabler som byggargument; lokalt:
#   docker build --build-arg VY_KEY=$(od -An -v -tx1 dev.key | tr -d ' \n') .
# Saknas det skapas en tillfällig nyckel för bygget, och klienter pinnar då en
# ny nyckel per bygge. Fröet skrivs till en temporär fil och raderas i samma
# RUN-steg i byggstadiet; bara dev.key.pub följer med i bilden.

FROM rust:1.97-slim-bookworm AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
# rust-toolchain.toml pekar ut exakt 1.97.0; installera den verktygskedjan
# (rustup show gör det) INNAN targets läggs till, annars hamnar musl-targetet
# på bildens standardkedja och saknas när cargo byter till den pinnade.
COPY rust-toolchain.toml ./
RUN rustup toolchain install 1.97.0 --profile minimal --component rustfmt --component clippy \
    && rustup target add --toolchain 1.97.0 x86_64-unknown-linux-musl wasm32-unknown-unknown \
    && rustup target list --toolchain 1.97.0 --installed
COPY . .
ENV CC_x86_64_unknown_linux_musl=musl-gcc CARGO_NET_RETRY=10 CARGO_BUILD_JOBS=4
# Diagnostik i loggen om något steg faller: vilken kedja och vilka targets som används.
RUN rustup show && cargo --version && rustc --print sysroot && ls "$(rustc --print sysroot)/lib/rustlib"
# Servern som statisk binär; publiceringsverktygen körs bara här i bygget.
RUN cargo build --release --locked --target x86_64-unknown-linux-musl -p vy-serve
RUN cargo build --release --locked -p vy-cli -p vy-publish
ARG VY_KEY=
RUN set -eu; \
    key=""; \
    if [ -n "$VY_KEY" ]; then \
        umask 077; mkdir -p /tmp/key; printf '%s\n' "$VY_KEY" > /tmp/key/dev.key; \
        key=/tmp/key/dev.key; \
    fi; \
    tools/publish_site.sh /srv $key; \
    rm -rf /tmp/key

FROM scratch
COPY --from=build /src/target/x86_64-unknown-linux-musl/release/vy-serve /vy-serve
COPY --from=build /srv /srv
ENV VY_DIR=/srv VY_PUBLIC_KEY=/srv/dev.key.pub PORT=8080
EXPOSE 8080
ENTRYPOINT ["/vy-serve"]
