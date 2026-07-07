FROM node:22-bookworm

ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:${PATH}
ENV RUST_TOOLCHAIN=1.95.0
ENV CARGO_LLVM_COV_VERSION=0.8.7

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      build-essential \
      ca-certificates \
      curl \
      pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y \
      --profile minimal \
      --default-toolchain ${RUST_TOOLCHAIN} \
    && rustup component add rustfmt \
    && rustup component add llvm-tools-preview \
    && rustup target add wasm32-unknown-unknown \
    && chmod -R a+w "${RUSTUP_HOME}" "${CARGO_HOME}"

COPY Cargo.toml Cargo.lock /tmp/warmup/
RUN cd /tmp/warmup && cargo fetch && rm -rf /tmp/warmup

COPY Cargo.lock /tmp/metadata-checker-Cargo.lock

RUN WASM_BINDGEN_VERSION="$(awk '\
      $0 == "name = \"wasm-bindgen\"" { found = 1; next } \
      found && $1 == "version" { gsub(/\"/, "", $3); print $3; exit } \
    ' /tmp/metadata-checker-Cargo.lock)" \
    && test -n "${WASM_BINDGEN_VERSION}" \
    && cargo install wasm-bindgen-cli --version "${WASM_BINDGEN_VERSION}" --locked \
    && wasm-bindgen --version \
    && rm -rf /tmp/metadata-checker-Cargo.lock /usr/local/cargo/registry /usr/local/cargo/git

RUN curl --proto '=https' --tlsv1.2 -sSfL https://bencher.dev/download/install-cli.sh | sh \
    && bencher --version

RUN cargo install cargo-llvm-cov --version "${CARGO_LLVM_COV_VERSION}" --locked \
    && cargo llvm-cov --version \
    && rm -rf /usr/local/cargo/registry /usr/local/cargo/git
