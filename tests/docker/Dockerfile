# syntax=docker/dockerfile:1
FROM ghcr.io/greenroom-robotics/pixi:0.82.2-bookworm AS base
COPY --from=chef /pixi-chef /usr/local/bin/pixi-chef
COPY --from=harness /checked.sh /usr/local/bin/checked.sh
WORKDIR /app

FROM base AS planner
ARG MANIFEST=pixi.toml
ARG PREPARE_ARGS=
COPY . .
RUN pixi-chef prepare "$MANIFEST" /recipe $PREPARE_ARGS

FROM base AS cook
ARG SEED
ARG ENV=default
ARG WORKSPACE_DIR=.
COPY --from=planner /recipe /app
WORKDIR /app/$WORKSPACE_DIR
RUN : "$SEED" && pixi-chef cook -e "$ENV"

FROM cook AS build
ARG ENV=default
COPY . /app
RUN checked.sh pixi-chef bake -e "$ENV" --export /export

FROM cook AS dev
ARG ENV=default
COPY . /app
RUN checked.sh pixi install --locked -e "$ENV"

FROM debian:bookworm-slim AS runtime
RUN rm /bin/bash && ! command -v bash
COPY --from=build /export /
ENTRYPOINT ["/entrypoint.sh"]
