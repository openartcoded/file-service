## RUNTIME
FROM alpine:3.22 AS runtime

RUN apk add --no-cache tzdata
RUN ln -s /usr/share/zoneinfo/Europe/Brussels /etc/localtime
RUN apk add --no-cache ca-certificates libreoffice chromium
RUN apk add --no-cache \
       --repository http://dl-cdn.alpinelinux.org/alpine/edge/testing \
       --repository http://dl-cdn.alpinelinux.org/alpine/edge/main \
       gperftools-dev

## INITIAL BUILDER
FROM rust:1.91-alpine3.22 AS builder
RUN apk add --no-cache openssl-dev curl build-base cmake pkgconfig musl-dev openssl-libs-static perl \
    && apk add --no-cache \
       --repository http://dl-cdn.alpinelinux.org/alpine/edge/testing \
       --repository http://dl-cdn.alpinelinux.org/alpine/edge/main \
       gperftools-dev

## CACHE LAYER
FROM builder AS cache
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
ENV RUSTFLAGS="-C link-args=-ltcmalloc"
RUN cargo build --release 
RUN rm -rf src

## BUILD LAYER
FROM cache AS build
WORKDIR /app
ENV RUSTFLAGS="-C link-args=-ltcmalloc"
COPY ./src ./src
RUN cargo build --release

## ENTRY POINT
FROM runtime
WORKDIR /app
COPY --from=build /app/target/release/file-service /file-service

ENV RUST_LOG=INFO
ENV LD_PRELOAD=/usr/lib/libtcmalloc.so
ENV TCMALLOC_AGGRESSIVE_DECOMMIT=t

ENTRYPOINT ["/file-service"]
