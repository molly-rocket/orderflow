# syntax=docker/dockerfile:1

FROM node:24.21.0-bookworm-slim AS web
RUN npm install --global pnpm@12.11.2
WORKDIR /src/web
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm run build


FROM rust:1.97.1-slim-bookworm AS build
WORKDIR /src
COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
        --package orderflow-collector \
        --package orderflow-writer \
        --package orderflow-gateway \
 && mkdir -p /out \
 && cp target/release/orderflow-collector target/release/orderflow-writer \
       target/release/orderflow-gateway /out/

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install --yes --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 orderflow
COPY --from=build /out/ /usr/local/bin/
COPY --from=web /src/web/dist /app/web
USER orderflow
