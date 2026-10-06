FROM rust:1.98.1-alpine AS builder

RUN apk add --no-cache musl-dev ca-certificates

WORKDIR /work

COPY . .

RUN cargo build --release && \
    cp target/release/deep-research-agent /deep-research-agent

FROM scratch

COPY --from=builder /deep-research-agent /deep-research-agent
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt

USER 1000:1000

ENTRYPOINT ["/deep-research-agent"]
