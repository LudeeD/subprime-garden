# syntax=docker/dockerfile:1

# Alpine's toolchain is musl-native, so the host target already is
# x86_64/aarch64-unknown-linux-musl — no cross-compilation needed. We still
# pass --target explicitly (rather than relying on the ambient default) so
# that the crt-static rustflags in .cargo/config.toml, which are scoped to
# that target triple, apply only to target artifacts and not to proc-macros/
# build-scripts (which cargo always compiles for the host); a blanket
# RUSTFLAGS env var breaks proc-macro compilation on musl.
FROM rust:1-alpine AS chef
RUN apk add --no-cache musl-dev build-base
RUN cargo install cargo-chef --locked
WORKDIR /app

ARG TARGETARCH
RUN case "$TARGETARCH" in \
      amd64) echo x86_64-unknown-linux-musl > /rust_target ;; \
      arm64) echo aarch64-unknown-linux-musl > /rust_target ;; \
      *) echo "unsupported arch: $TARGETARCH" >&2; exit 1 ;; \
    esac

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
COPY .cargo /app/.cargo
# Dependency-only build: this layer only invalidates when Cargo.toml/Cargo.lock
# change, not on every template or handler edit.
RUN cargo chef cook --release --target "$(cat /rust_target)" --recipe-path recipe.json
COPY . .
RUN cargo build --release --target "$(cat /rust_target)" --bin subprime-garden \
    && cp "target/$(cat /rust_target)/release/subprime-garden" /subprime-garden

# distroless/static has no libc, no shell, no package manager — a fully
# static musl binary is the only thing that runs here, which is the point.
FROM gcr.io/distroless/static-debian12:latest
COPY --from=builder /subprime-garden /subprime-garden

USER 1000:1000
VOLUME ["/data"]
EXPOSE 8080
ENV SUBPRIME_SERVER__BIND=0.0.0.0:8080

ENTRYPOINT ["/subprime-garden"]
CMD ["serve"]
