FROM rust:1.88.0-slim AS chef
# 安装 cargo-chef
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

# 生成缓存 recipe
RUN cargo chef prepare --recipe-path recipe.json

# 使用缓存构建依赖
RUN cargo chef cook --recipe-path recipe.json

# === 第二阶段：实际构建 ===
FROM planner AS builder

WORKDIR /app

# 复制缓存和依赖
COPY --from=planner /app/recipe.json recipe.json
COPY Cargo.toml Cargo.lock ./

# 复制 workspace 成员目录
COPY api_tool ./api_tool/
COPY route_codegen ./route_codegen/
COPY src ./src/


# 构建最终可执行文件并输出日志到文件
RUN cargo build --release


# === 第三阶段：最终运行镜像 ===
FROM  debian:latest
RUN apt update && apt install -y apt

# 安装 curl
RUN apt-get install -y curl && \
    rm -rf /var/lib/apt/lists/*


WORKDIR /app

# 复制构建产物
#Copy binary from the previous stage
COPY --from=builder /app/target/release/web_demo  .


#Actix port
EXPOSE 8080

#Start app
ENTRYPOINT ["./web_demo"]

