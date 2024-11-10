FROM rust:1.82 AS chef 
RUN apt-get update && apt-get upgrade -y
RUN apt-get install -y libssl-dev build-essential cmake

RUN cargo install cargo-chef 

WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare  --recipe-path recipe.json

FROM chef AS builder

COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application

COPY . .
RUN cargo build --release

FROM debian:bookworm-slim AS runtime
RUN apt update && apt upgrade -y
RUN apt install -y ca-certificates
RUN apt install  --no-install-recommends -y libreoffice chromium

FROM runtime
WORKDIR /app
COPY --from=builder /app/target/release/kofte-rs /kofte
ENV RUST_LOG=INFO
ENV TZ="Europe/Brussels"
ENTRYPOINT  ["/kofte"]
