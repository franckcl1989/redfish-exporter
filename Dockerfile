# syntax=docker/dockerfile:1
FROM rust:1.90.0-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl

FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/redfish-exporter /redfish-exporter
EXPOSE 9417
ENTRYPOINT ["/redfish-exporter"]
