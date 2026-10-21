# pixi-chef

Splits a pixi workspace's container build into a dependency layer that only changes with the lock, and a source layer that changes with every edit.

- `prepare` writes the **recipe**: the manifest, the lock and the metadata of each **source package**.
- `cook` installs the environment's **binary packages** from the recipe. Its layer is rebuilt only when the recipe changes.
- `bake` installs the **source packages** on top of the cooked layer and writes an **export** that can be copied onto a runtime image.

## Install

On a workstation, with the Greenroom pixi fork and an `az login` session (see the Greenroom development docs, "Install" and "Auth"):

```bash
pixi global install \
  -c az://stgrcondachannel.blob.core.windows.net/general \
  -c conda-forge \
  pixi-chef
```

In a Dockerfile, copy the binary from the image. It is a `scratch` image holding one static binary at `/pixi-chef`:

```dockerfile
COPY --from=ghcr.io/greenroom-robotics/pixi-chef:<version> /pixi-chef /usr/local/bin/pixi-chef
```

The `ghcr.io` package is private. Whoever builds the Dockerfile needs read access to it and must be logged in to `ghcr.io`.

pixi-chef runs `pixi`, so `pixi` must be on `PATH` wherever `cook` or `bake` run.

## Language

- **Source package**: a lock entry built from a local directory rather than fetched as a built artifact.
- **Binary package**: a lock entry installed from a built artifact, wherever that artifact lives, including a `.conda` file on local disk.
- **Recipe**: the trimmed copy of a repo holding only the workspace manifest, its lock, and the metadata files of each source package, laid out at their real paths.
- **Into directory**: the directory `prepare --into` places the manifest and lock at. `cook` and `bake` both run there, so `PIXI_PROJECT_ROOT` is that directory at runtime.
- **Export**: a tree laid out as a filesystem root that holds one installed environment, every source package's directory, the activation hook and the entrypoint, ready to copy onto a runtime image.

## Commands

### `pixi-chef prepare <manifest> <out> [--into <dir>] [--include <glob>...]`

Run from the repo root. Writes the recipe to `<out>`.

- `<manifest>` must be a file named `pixi.toml` inside the current directory. The lock beside it must exist.
- `<out>` must be absent or empty.
- The recipe holds the manifest, `pixi.lock`, and, for each source package in the lock, the files of its directory that name it: `pixi.toml`, `pixi.lock`, `package.xml`, `pyproject.toml`, `setup.py`, `setup.cfg`, `CMakeLists.txt` and `README.md`. Everything is at its real path.
- The recipe depends only on those files, so editing source code does not change it.
- `--into <dir>` places the manifest and lock at `<dir>` (a path inside the repo) instead of beside the manifest. It is accepted only when every lock path, activation script and `build-variants-files` path resolves to the same place from `<dir>` as from the manifest's directory. Otherwise it is an error naming the path.
- `--include <glob>` copies more files from each source package directory into the recipe. It can be repeated. Globs are relative to the package directory:
  - `*` does not cross `/`; use `**` to cross directories.
  - A glob that matches a directory takes everything under it.
  - Gitignored files are skipped.
  - Absolute patterns and patterns containing `..` are rejected.
  - A glob that matches nothing is an error.

Each source package's `pixi.toml` needs its own `[workspace]` table. pixi 0.82 rejects a package-only manifest that sits outside its workspace tree, and `prepare` does not copy ancestor workspace manifests.

### `pixi-chef cook -e <environment>`

Run in the directory holding the recipe (the **into directory** when `--into` was used). Installs the environment's binary packages with the lock as is, and skips the source packages. It installs for the current platform and exits with pixi's exit code.

### `pixi-chef bake -e <environment> --export <dir>`

Run in the directory holding the recipe with the real sources copied over it. Installs the environment from the lock (the cooked binary packages are not reinstalled) and writes the export to `<dir>`.

- `<dir>` must be absent or empty, and must not be a mount point.
- The environment must include `bash`. `bake` fails otherwise.
- The export holds, at their real paths: the environment, each source package's directory, and the activation scripts that the environment's hook sources.
- It also holds `/etc/profile.d/pixi-env.sh`, the environment's activation hook, and `/entrypoint.sh`.
- `/entrypoint.sh` is a `#!/bin/sh` script. It runs the environment's own `bash`, sources the hook, then execs its arguments. The runtime image needs no system `bash`.
- `bake` fails when an activation script the manifest names is missing.

Conda prefixes are absolute, so the workspace must sit at the same path in the build stages and in the runtime image. The examples use `/app`.

## Not handled

pixi-chef does not manage:

- Secrets. Pass credentials to `cook` and `bake` through your build system.
- pixi itself. Provide a `pixi` that can read your channels.
- Channels. They come from the manifest and pixi's own config.
- The runtime image's `ENV`, `USER` and file ownership.

## Examples

All examples build a workspace laid out as below. `pixi.lock` is committed and a `.dockerignore` lists `.pixi`.

```
pixi.toml        bash and hello = { path = "hello" }
pixi.lock
hello/pixi.toml  has its own [workspace] table
hello/pyproject.toml
hello/hello/__init__.py
```

### Minimal production image

```dockerfile
# syntax=docker/dockerfile:1
ARG PIXI_CHEF_VERSION
FROM ghcr.io/greenroom-robotics/pixi-chef:${PIXI_CHEF_VERSION} AS chef
FROM ghcr.io/prefix-dev/pixi:0.82.1 AS tools
COPY --from=chef /pixi-chef /usr/local/bin/pixi-chef

FROM tools AS recipe
WORKDIR /src
COPY . .
RUN pixi-chef prepare pixi.toml /recipe

FROM tools AS cook
WORKDIR /app
COPY --from=recipe /recipe .
RUN pixi-chef cook -e default

FROM cook AS bake
COPY . .
RUN pixi-chef bake -e default --export /export

FROM debian:bookworm-slim
COPY --from=bake /export/ /
WORKDIR /app
ENTRYPOINT ["/entrypoint.sh"]
```

The `cook` stage depends only on the recipe, so BuildKit reuses its layer until the manifest, the lock or a source package's metadata changes. Editing source code re-runs only `bake`.

### Private channels

Give `pixi` its credentials for `cook` (and `bake`) with a BuildKit secret, so they never enter a layer. Replace the `cook` stage of the minimal example:

```dockerfile
FROM tools AS cook
WORKDIR /app
COPY --from=recipe /recipe .
RUN --mount=type=secret,id=rattler-auth,target=/run/secrets/rattler-auth.json \
    RATTLER_AUTH_FILE=/run/secrets/rattler-auth.json \
    pixi-chef cook -e default
```

```bash
docker build --secret id=rattler-auth,src=auth.json .
```

`auth.json` is a rattler authentication file. Repeat the mount on the `bake` `RUN`. The base image's `pixi` must support your channel scheme: for the Greenroom `az://` channel that is the Greenroom pixi fork.

### Named environment

Cook and bake the environment by name. Only the packages of that environment are installed and exported.

```dockerfile
RUN pixi-chef cook -e prod
...
RUN pixi-chef bake -e prod --export /export
```

### Dev image

Start from the `cook` stage and install the source packages in place. The image keeps `pixi`.

```dockerfile
FROM cook AS dev
COPY . .
RUN pixi install --locked -e default
ENTRYPOINT ["pixi", "run", "-e", "default"]
```

```bash
docker build --target dev -t app-dev .
```

### Two environments in one image

Cook both environments in one layer, bake each to its own export, then copy the first export whole and only the second environment's directory:

```dockerfile
FROM tools AS cook
WORKDIR /app
COPY --from=recipe /recipe .
RUN pixi-chef cook -e prod && pixi-chef cook -e tools

FROM cook AS bake
COPY . .
RUN pixi-chef bake -e prod --export /export/prod \
 && pixi-chef bake -e tools --export /export/tools

FROM debian:bookworm-slim
COPY --from=bake /export/prod/ /
COPY --from=bake /export/tools/app/.pixi/envs/tools /app/.pixi/envs/tools
WORKDIR /app
ENTRYPOINT ["/entrypoint.sh"]
```

Both exports write `/entrypoint.sh` and the hook at the same path, so only one export is copied whole. Run the second environment's programs by their path, `/app/.pixi/envs/tools/bin/<program>`.

### `--into`: one cook layer for many manifests

When several manifests under `variants/` have the same dependencies, `--into` gives their recipes the same layout, so variants with the same lock share one `cook` layer. This needs every path in the manifest and lock to resolve the same from `variants/shared` as from `variants/<name>`, for example `../../common/hello`.

```dockerfile
# syntax=docker/dockerfile:1
ARG PIXI_CHEF_VERSION
ARG VARIANT=a
FROM ghcr.io/greenroom-robotics/pixi-chef:${PIXI_CHEF_VERSION} AS chef
FROM ghcr.io/prefix-dev/pixi:0.82.1 AS tools
COPY --from=chef /pixi-chef /usr/local/bin/pixi-chef

FROM tools AS recipe
ARG VARIANT
WORKDIR /src
COPY . .
RUN pixi-chef prepare variants/${VARIANT}/pixi.toml /recipe --into variants/shared

FROM tools AS cook
WORKDIR /app/variants/shared
COPY --from=recipe /recipe /app
RUN pixi-chef cook -e default

FROM cook AS bake
COPY . /app
RUN pixi-chef bake -e default --export /export

FROM debian:bookworm-slim
COPY --from=bake /export/ /
WORKDIR /app/variants/shared
ENTRYPOINT ["/entrypoint.sh"]
```

`cook` and `bake` run in `/app/variants/shared`, so `PIXI_PROJECT_ROOT` is `/app/variants/shared` in the running container.
