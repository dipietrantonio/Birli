FROM ubuntu:21.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y \
        aoflagger-dev \
        build-essential \
        curl \
        git \
        jq \
        libcfitsio-dev \
        liberfa-dev \
        libssl-dev \
        pkg-config \
        unzip \
        zip \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# Get Rust
RUN mkdir -m755 /opt/rust /opt/cargo
ENV RUSTUP_HOME=/opt/rust CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y

# Get cargo make
RUN cargo install --force cargo-make cargo-binutils

ADD . /app
WORKDIR /app

RUN cargo clean
RUN cargo install --path . --features aoflagger

ENTRYPOINT [ "birli" ]
