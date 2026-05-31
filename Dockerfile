# syntax=docker/dockerfile:1.7
FROM rust:1.87-alpine AS deps

RUN apk add --no-cache musl-dev ca-certificates

WORKDIR /app
ARG BUILD_PROFILE=release

COPY Cargo.toml Cargo.lock ./

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    mkdir -p src/bin \
    && printf '' > src/lib.rs \
    && printf 'fn main() {}\n' > src/main.rs \
    && printf 'fn main() {}\n' > src/bin/result_agent.rs \
    && if [ "$BUILD_PROFILE" = "release" ]; then \
        cargo_profile_args="--release"; profile_dir="release"; \
      elif [ "$BUILD_PROFILE" = "dev" ]; then \
        cargo_profile_args="--profile dev"; profile_dir="debug"; \
      else \
        cargo_profile_args="--profile $BUILD_PROFILE"; profile_dir="$BUILD_PROFILE"; \
      fi \
    && cargo build $cargo_profile_args --locked \
        --bin danske-spil-gambler \
        --bin danske-spil-result-agent \
    && rm -rf src \
        "target/$profile_dir/danske-spil-gambler" \
        "target/$profile_dir/danske-spil-result-agent" \
        target/"$profile_dir"/libdanske_spil_gambler* \
        target/"$profile_dir"/deps/danske_spil_gambler-* \
        target/"$profile_dir"/deps/danske_spil_result_agent-* \
        target/"$profile_dir"/deps/libdanske_spil_gambler-* \
        target/"$profile_dir"/.fingerprint/danske-spil-gambler-* \
        target/"$profile_dir"/.fingerprint/danske-spil-result-agent-*

FROM deps AS builder
ARG BUILD_PROFILE=release
COPY src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    if [ "$BUILD_PROFILE" = "release" ]; then \
        cargo_profile_args="--release"; profile_dir="release"; \
    elif [ "$BUILD_PROFILE" = "dev" ]; then \
        cargo_profile_args="--profile dev"; profile_dir="debug"; \
    else \
        cargo_profile_args="--profile $BUILD_PROFILE"; profile_dir="$BUILD_PROFILE"; \
    fi \
    && cargo build $cargo_profile_args --locked \
        --bin danske-spil-gambler \
        --bin danske-spil-result-agent \
    && cp "target/$profile_dir/danske-spil-gambler" /out-gambler \
    && cp "target/$profile_dir/danske-spil-result-agent" /out-result-agent

FROM scratch

ENV GAMBLER_HOST=0.0.0.0 \
    GAMBLER_PORT=8080

COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /out-gambler /gambler
COPY --from=builder /out-result-agent /result-agent

EXPOSE 8080

ENTRYPOINT ["/gambler"]
