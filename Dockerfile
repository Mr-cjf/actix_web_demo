FROM rust:1.88.0-slim AS chef
WORKDIR /app
# 安装 cargo-chef 和 musl 工具链
RUN apt-get update && apt-get install -y \
    musl-tools \
    && rustup target add x86_64-unknown-linux-musl
RUN cargo install --locked cargo-chef

# === 第一阶段：生成缓存 recipe ===
FROM chef AS planner

WORKDIR /app

# 复制依赖描述文件
COPY Cargo.toml Cargo.lock ./

# 复制 workspace 成员目录
COPY api_tool ./api_tool/
COPY route_codegen ./route_codegen/
COPY src ./src/

RUN cargo clean

# 生成缓存 recipe 并写入日志
RUN echo "【Chef 阶段】准备构建缓存..." > /tmp/build.log && \
    cargo chef prepare --recipe-path recipe.json >> /tmp/build.log 2>&1 && \
    echo "【Chef 阶段】缓存生成完成" >> /tmp/build.log


# 打印构建日志
RUN cat /tmp/build.log

# === 第二阶段：实际构建 ===
FROM chef AS builder

WORKDIR /app

# 复制缓存和依赖
COPY --from=planner /app/recipe.json recipe.json
COPY --from=planner /tmp/build.log /tmp/build.log
COPY Cargo.toml Cargo.lock ./

# 复制 workspace 成员目录
COPY api_tool ./api_tool/
COPY route_codegen ./route_codegen/
COPY src ./src/
COPY .cargo/config.toml /root/.cargo/config.toml


# 使用缓存构建依赖并写入日志
RUN echo "【Builder 阶段】开始使用缓存构建依赖..." >> /tmp/build.log && \
    cargo chef cook --release --recipe-path recipe.json >> /tmp/build.log 2>&1 && \
    echo "【Builder 阶段】依赖构建完成" >> /tmp/build.log

# 构建前检查目录结构并记录
RUN echo "【Builder 阶段】当前工作目录内容：" >> /tmp/build.log && \
    ls -la /app >> /tmp/build.log && \
    echo "【Builder 阶段】检查 workspace 成员目录：" >> /tmp/build.log && \
    ls -la /app/api_tool /app/route_codegen  >> /tmp/build.log && \
    echo "【Builder 阶段】开始构建可执行文件..." >> /tmp/build.log

# 构建最终可执行文件并输出日志到文件
RUN echo "cargo build --release 开始..." >> /tmp/build.log && \
    cargo build --release --target=x86_64-unknown-linux-musl >> /tmp/build.log 2>&1 && \
    echo "cargo build --release --target=x86_64-unknown-linux-musl 完成" >> /tmp/build.log && \
    echo "构建产物列表：" >> /tmp/build.log && \
    ls -la /app/target/release/ >> /tmp/build.log && \
    ls -la /app/target/x86_64-unknown-linux-musl/release/ >> /tmp/build.log

# 打印构建日志
RUN cat /tmp/build.log

# === 第三阶段：最终运行镜像 ===
FROM alpine:latest

WORKDIR /app

# 安装必要依赖（如证书、file 工具）
RUN apk add --no-cache \
    file \
    openssl \
    ca-certificates

# 复制构建产物
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/web_demo /app/web_demo
COPY --from=builder /tmp/build.log /app/logs/build.log

# 确保可执行权限
RUN chmod +x /app/web_demo

# 暴露端口
EXPOSE 8080

# 启动命令
CMD ["/app/web_demo"]

