FROM scratch
LABEL org.opencontainers.image.source=https://github.com/greenroom-robotics/pixi-chef
ARG TARGETARCH
COPY dist/${TARGETARCH}/pixi-chef /pixi-chef
ENTRYPOINT ["/pixi-chef"]
