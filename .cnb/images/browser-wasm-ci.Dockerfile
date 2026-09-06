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
      openssh-server \
      pkg-config \
    && rm -rf /var/lib/apt/lists/*

# CNB 自定义开发环境要走 VSCode/Cursor Remote-SSH，必须在镜像里预装 openssh-server
# （见上面的 apt 列表）；sshd 需要这个运行目录，缺了会在连接时才报错。
RUN mkdir -p /run/sshd

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y \
      --profile minimal \
      --default-toolchain ${RUST_TOOLCHAIN} \
    && rustup component add rustfmt \
    && rustup component add llvm-tools-preview \
    && rustup target add wasm32-unknown-unknown \
    && chmod -R a+w "${RUSTUP_HOME}" "${CARGO_HOME}"

# CNB docker.build.by 只放进白名单文件；manifest 声明了 bin/bench 目标，
# cargo fetch 解析时要求对应源文件存在，因此用空 stub 满足路径检查。
COPY Cargo.toml Cargo.lock /tmp/warmup/
RUN cd /tmp/warmup \
    && mkdir -p src/bin benches \
    && printf '\n' > src/lib.rs \
    && printf 'fn main() {}\n' > src/main.rs \
    && printf 'fn main() {}\n' > src/bin/generate_browser_offscreen_fixtures.rs \
    && printf 'fn main() {}\n' > src/bin/m51_profile_report.rs \
    && printf 'fn main() {}\n' > src/bin/m51_core_profile_report.rs \
    && for bench in \
         parse_bench \
         query_micro_bench \
         rebuild_bench \
         rebuild_crud_bench \
         runtime_bench \
         query_matrix_bench \
         redb_persistence_bench \
         stdio_boundary_bench \
         telemetry_overhead_bench \
         session_sync_bench \
       ; do printf 'fn main() {}\n' > "benches/${bench}.rs"; done \
    && cargo fetch \
    && rm -rf /tmp/warmup

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
