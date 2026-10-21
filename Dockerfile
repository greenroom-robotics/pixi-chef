FROM scratch
ARG TARGETARCH
COPY dist/${TARGETARCH}/pixi-chef /pixi-chef
ENTRYPOINT ["/pixi-chef"]
