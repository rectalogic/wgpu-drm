FROM rust:latest

RUN rustup component add clippy rustfmt
# RUN apk update && apk add wayland alsa-lib-dev libudev-zero-dev
RUN apt-get update && apt-get install -y libwayland-dev libasound2-dev libudev-dev
