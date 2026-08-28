# syntax=docker/dockerfile:1
# Multi-platform manifest digests pinned for reproducible, reviewable supply-chain inputs.
FROM rust:1.90.0-alpine@sha256:b4b54b176a74db7e5c68fdfe6029be39a02ccbcfe72b6e5a3e18e2c61b57ae26 AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
RUN cargo build --release --locked --target x86_64-unknown-linux-musl

FROM gcr.io/distroless/static-debian12:nonroot@sha256:afa5c872c891853ca7fcf1f12c3edb23f7eeef36189728842dd51042ff57f7ab
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/redfish-exporter /redfish-exporter
USER nonroot:nonroot
EXPOSE 9417
ENTRYPOINT ["/redfish-exporter"]
