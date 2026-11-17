# Build from the repository root: docker build -f contrib/docker/Dockerfile .
FROM node:24-bookworm AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
COPY schema/tdo.openrpc.json /src/schema/tdo.openrpc.json
RUN npm run build

FROM rust:1.99-bookworm AS build
WORKDIR /src
COPY . .
COPY --from=web /src/web/dist web/dist
ENV TWYDO_REQUIRE_WEB=1
RUN cargo build --release --locked -p tdo

FROM debian:bookworm-slim
LABEL org.opencontainers.image.source="https://github.com/bwilczynski/twydo"
RUN apt-get update \
    && apt-get install -y --no-install-recommends tini ca-certificates python3 python3-venv \
    && rm -rf /var/lib/apt/lists/* \
    && python3 -m venv /opt/vdirsyncer \
    && /opt/vdirsyncer/bin/pip install --no-cache-dir vdirsyncer==0.21.0 \
    && ln -s /opt/vdirsyncer/bin/vdirsyncer /usr/local/bin/vdirsyncer \
    && useradd --uid 1000 --create-home --shell /usr/sbin/nologin twydo \
    && mkdir -p /data /run/twydo \
    && chown twydo:twydo /data /run/twydo \
    && chmod 0700 /run/twydo
COPY --from=build /src/target/release/tdo /usr/local/bin/tdo
COPY contrib/docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod 0755 /usr/local/bin/entrypoint.sh
ENV XDG_CONFIG_HOME=/data/config \
    XDG_STATE_HOME=/data/state \
    XDG_RUNTIME_DIR=/run/twydo \
    TWYDO_SOCKET=/run/twydo/tdo.sock \
    TWYDO_NOTIFIER=off \
    TWYDO_GATEWAY_LISTEN=0.0.0.0:8787
USER twydo
VOLUME /data
EXPOSE 8787
ENTRYPOINT ["tini", "--", "/usr/local/bin/entrypoint.sh"]
