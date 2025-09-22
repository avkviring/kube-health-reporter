# Install cargo-chef
FROM rust:1.89-bookworm AS chef
RUN cargo install cargo-chef
WORKDIR /app

# Plan dependencies
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Build dependencies
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Build application
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/kube-health-reporter /usr/local/bin/kube-health-reporter
ENTRYPOINT ["/usr/local/bin/kube-health-reporter"]


