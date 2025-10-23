FROM rust:1.90-alpine3.21 AS chef 

RUN  apk add --no-cache openssl-dev curl build-base cmake pkgconfig musl-dev  openssl-libs-static perl 
RUN apk add \
    --no-cache \
    --repository http://dl-cdn.alpinelinux.org/alpine/edge/testing \
    --repository http://dl-cdn.alpinelinux.org/alpine/edge/main \
    gperftools-dev
RUN cargo install cargo-chef 


WORKDIR /app

FROM chef AS planner
COPY ./Cargo.toml ./Cargo.lock ./
COPY ./src ./src
RUN cargo chef prepare  --recipe-path recipe.json

FROM chef AS builder

COPY --from=planner /app/recipe.json recipe.json

# Build dependencies - this is the caching Docker layer!
ENV RUSTFLAGS="-C link-args=-ltcmalloc" 
RUN  cargo chef cook --release --recipe-path recipe.json
# Build application

COPY . .

RUN cargo build --release

FROM alpine:3.21 AS runtime
RUN apk add --no-cache tzdata
RUN ln -s /usr/share/zoneinfo/Europe/Brussels /etc/localtime

RUN apk add --no-cache ca-certificates libreoffice chromium

FROM runtime

ARG USERNAME=file-service
ARG USER_UID=1000
ARG USER_GID=$USER_UID

WORKDIR /app
COPY --from=builder /app/target/release/file-service /file-service
ENV RUST_LOG=INFO
ENV LD_PRELOAD=/usr/lib/libtcmalloc.so
ENV TCMALLOC_AGGRESSIVE_DECOMMIT=t

ENTRYPOINT  ["/file-service"]
