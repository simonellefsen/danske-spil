FROM rust:1.87-alpine AS builder

RUN apk add --no-cache musl-dev ca-certificates

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked

FROM scratch

ENV GAMBLER_HOST=0.0.0.0 \
    GAMBLER_PORT=8080

COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /app/target/release/danske-spil-gambler /gambler

EXPOSE 8080

ENTRYPOINT ["/gambler"]
