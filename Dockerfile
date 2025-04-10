FROM rust:1.86 AS chef 
RUN apt-get update && apt-get upgrade -y
RUN apt-get install -y libssl-dev build-essential cmake

RUN cargo install cargo-chef 

# install mold
ENV MOLD_VERSION=2.37.1
RUN wget https://github.com/rui314/mold/releases/download/v${MOLD_VERSION}/mold-${MOLD_VERSION}-x86_64-linux.tar.gz \
    && tar -xvzf mold-${MOLD_VERSION}-x86_64-linux.tar.gz \
    && mv mold-${MOLD_VERSION}-x86_64-linux/bin/* /usr/local/bin

WORKDIR /app

FROM chef AS planner
COPY ./Cargo.toml ./Cargo.lock ./
COPY ./src ./src
RUN cargo chef prepare  --recipe-path recipe.json

FROM chef AS builder

COPY --from=planner /app/recipe.json recipe.json

# Build dependencies - this is the caching Docker layer!
RUN RUSTFLAGS="-C link-arg=-fuse-ld=mold"  cargo chef cook --release --recipe-path recipe.json
# Build application

COPY . .

RUN RUSTFLAGS="-C link-arg=-fuse-ld=mold" cargo build --release

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
