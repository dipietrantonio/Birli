FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive
ENV LANG=en_US.UTF-8 LANGUAGE=en_US:en LC_ALL=en_US.UTF-8
ARG DEBUG
RUN apt-get update \
    && apt-get install -y \
    aoflagger-dev \
    build-essential \
    clang \
    curl \
    git \
    jq \
    lcov \
    libcfitsio-dev \
    liberfa-dev \
    libssl-dev \
    pkg-config \
    unzip \
    zip \
    automake \
    libtool

RUN test -z "$DEBUG" || ( \
    apt-get install -y vim gdb \
    )
RUN apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# Get Rust
RUN mkdir -m755 /opt/rust /opt/cargo
ENV RUSTUP_HOME=/opt/rust CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
# install latest stable rust toolchian, with llvm-tools-preview (for coverage)
ENV RUST_VERSION=stable
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain=$RUST_VERSION
RUN rustup toolchain install $RUST_VERSION --component llvm-tools-preview
# Get cargo make, llvm-cov
RUN /opt/cargo/bin/cargo install --force cargo-make cargo-llvm-cov

ADD . /app
WORKDIR /app

RUN cargo clean \
    && cargo install --path . --features aoflagger --locked $(test -z "$DEBUG" || echo "--debug") \
    && cargo clean

ENTRYPOINT [ "/opt/cargo/bin/birli" ]
