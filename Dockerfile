# syntax=docker/dockerfile:1.7
FROM rust:1.87-alpine AS deps

RUN apk add --no-cache musl-dev ca-certificates

WORKDIR /app

COPY Cargo.toml Cargo.lock ./

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    mkdir -p src/bin \
    && printf '' > src/lib.rs \
    && printf 'fn main() {}\n' > src/main.rs \
    && printf 'fn main() {}\n' > src/bin/result_agent.rs \
    && cargo build --release --locked \
        --bin danske-spil-gambler \
        --bin danske-spil-result-agent \
    && rm -rf src \
        target/release/danske-spil-gambler \
        target/release/danske-spil-result-agent \
        target/release/libdanske_spil_gambler* \
        target/release/deps/danske_spil_gambler-* \
        target/release/deps/danske_spil_result_agent-* \
        target/release/deps/libdanske_spil_gambler-* \
        target/release/.fingerprint/danske-spil-gambler-* \
        target/release/.fingerprint/danske-spil-result-agent-*

FROM deps AS builder
COPY src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked \
        --bin danske-spil-gambler \
        --bin danske-spil-result-agent \
    && cp target/release/danske-spil-gambler /out-gambler \
    && cp target/release/danske-spil-result-agent /out-result-agent

FROM scratch

ENV GAMBLER_HOST=0.0.0.0 \
    GAMBLER_PORT=8080

COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /out-gambler /gambler
COPY --from=builder /out-result-agent /result-agent

EXPOSE 8080

ENTRYPOINT ["/gambler"]
