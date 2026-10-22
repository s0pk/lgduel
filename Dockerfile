# LGDuel dedicated server.
#   docker build -t lgduel-server .
#   docker run -d --name lgduel -p 27960:27960/udp --restart unless-stopped lgduel-server
# Config: mount your own file over /etc/lgduel/server.toml (see config/server.example.toml).
FROM rust:1.99.1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p lgduel-server

FROM debian:bookworm-slim
RUN useradd --system --no-create-home --shell /usr/sbin/nologin lgduel
COPY --from=build /src/target/release/lgduel-server /usr/local/bin/lgduel-server
COPY config/server.example.toml /etc/lgduel/server.toml
USER lgduel
EXPOSE 27960/udp
ENTRYPOINT ["lgduel-server", "--config", "/etc/lgduel/server.toml"]
