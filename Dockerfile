ARG APP_NAME=multiplayer-game-demo-rust

FROM rust:1.81-bullseye as builder
WORKDIR /usr/src/${APP_NAME}
COPY . .
RUN cargo install --path .

FROM debian:bullseye-slim
COPY --from=builder /usr/local/cargo/bin/${APP_NAME} /usr/local/bin/${APP_NAME}
ENTRYPOINT ["multiplayer-game-demo-rust", "--server-only", "--trace"]

