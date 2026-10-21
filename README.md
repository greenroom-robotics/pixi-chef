# pixi-chef

Splits a pixi workspace's container build into a dependency layer and a source layer, so editing source code does not reinstall the binary packages.

- `prepare` writes the **recipe**: the manifest, the lock and the metadata of each **source package**.
- `cook` installs the environment's **binary packages** from the recipe. Its layer can be reused while the recipe, the base image and pixi-chef are unchanged.
- `bake` installs the **source packages** on top of the cooked layer and writes an **export** that can be copied onto a runtime image.

## Install

On a workstation, with the Greenroom pixi fork, an `az login` session and a user-config grant for the `general` container (see the Install and Auth pages of the Greenroom platform docs: `website/docs_base/development/install.md` and `auth.md` in the platform repo):

```bash
pixi global install \
  -c az://stgrcondachannel.blob.core.windows.net/general \
  -c conda-forge \
  pixi-chef
```

In a Dockerfile, copy the binary from the image that `.github/workflows/image.yml` publishes. It is a `scratch` image holding one static binary at `/pixi-chef`:

```dockerfile
COPY --from=ghcr.io/greenroom-robotics/pixi-chef:<version> /pixi-chef /usr/local/bin/pixi-chef
```

The `ghcr.io` package is private. Whoever builds the Dockerfile needs read access to it and must be logged in to `ghcr.io`.

`cook` and `bake` run `pixi`, so `pixi` must be on `PATH` wherever they run.

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
- The recipe holds the manifest, `pixi.lock`, and, for each source package in the lock, these files, when present in its directory: `pixi.toml`, `pixi.lock`, `package.xml`, `pyproject.toml`, `setup.py`, `setup.cfg`, `CMakeLists.txt` and `README.md`. Everything is at its real path.
- Unless `--include` adds them, the recipe holds no source files, so editing source code does not change it.
- `--into <dir>` places the manifest and lock at `<dir>` (a path inside the repo) instead of beside the manifest. It is accepted only when every lock path, activation script and `build-variants-files` path resolves to the same place from `<dir>` as from the manifest's directory. Otherwise it is an error naming the path.
- `--include <glob>` copies more files from each source package directory into the recipe. It can be repeated. Globs are relative to the package directory:
  - `*` does not cross `/`; use `**` to cross directories.
  - A glob that matches a directory takes everything under it.
  - Gitignored files are skipped.
  - Absolute patterns and patterns containing `..` are rejected.
  - A glob that matches nothing is an error.

A source package whose directory is outside the manifest's directory needs its own `[workspace]` table in its `pixi.toml`, because `prepare` does not copy ancestor manifests.

### `pixi-chef cook -e <environment>`

Run in the directory holding the recipe (the **into directory** when `--into` was used). Installs the environment's binary packages with the lock as is, and skips the source packages. It installs for the current platform and exits with pixi's exit code.

### `pixi-chef bake -e <environment> --export <dir>`

Run in the directory holding the recipe with the real sources copied over it. Installs the environment from the lock (the cooked binary packages are not reinstalled) and writes the export to `<dir>`.

- `<dir>` must be absent or empty, and must not be a mount point.
- The environment must include `bash`. `bake` fails otherwise.
- The export holds, at their real paths: the environment, each source package's directory, and the activation scripts that the environment's hook sources.
- It also holds `/etc/profile.d/pixi-env.sh`, the environment's activation hook, and `/entrypoint.sh`.
- `/entrypoint.sh` is a `#!/bin/sh` script. It runs the environment's own `bash`, sources the hook, then execs its arguments. The runtime image needs `/bin/sh` and glibc, but no system `bash`. `FROM scratch` does not work.
- `bake` fails when an activation script that the manifest names for that environment and platform is missing.

Conda prefixes are absolute, so the workspace must sit at the same path in the build stages and in the runtime image. The examples use `/app`.

## Not handled

pixi-chef does not manage:

- Secrets. Pass credentials to `cook` and `bake` through your build system.
- pixi itself. Provide a `pixi` that can read your channels.
- Channels. They come from the manifest and pixi's own config.
- The runtime image's `ENV`, `USER` and file ownership.

## Examples

Unless an example says otherwise, it builds this workspace. `pixi.lock` is committed and a `.dockerignore` lists `**/.pixi`.

```
pixi.toml        depends on bash and hello = { path = "hello" }
pixi.lock
hello/pixi.toml
hello/pyproject.toml
hello/hello/__init__.py
```

The named-environment and two-environment examples add `[environments]` to `pixi.toml`, with a `prod` environment and a `tools` environment whose feature adds `jq`. Both include the default feature, so both include `bash`. The `--into` example has its own layout.

Every example takes the pixi-chef version as a build argument:

```bash
docker build --build-arg PIXI_CHEF_VERSION=<version> -t app .
docker run --rm app <program>
```

### Minimal production image

```dockerfile
# syntax=docker/dockerfile:1
ARG PIXI_CHEF_VERSION
FROM ghcr.io/greenroom-robotics/pixi-chef:${PIXI_CHEF_VERSION} AS chef
FROM ghcr.io/prefix-dev/pixi:0.82.0 AS tools
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

BuildKit reuses the `cook` layer while the recipe is byte-identical, and the base image and pixi-chef are unchanged. `prepare` re-runs on every source edit, but the recipe it writes does not change, so `cook` stays cached and only `bake` re-runs.

### Private channels

#### Greenroom `az://` channel

`az://` channels need the Greenroom pixi fork; upstream pixi cannot resolve them. The fork reads its credential from Azure environment variables, and only for containers granted in the user-level (`~/.pixi/config.toml`) or system-level pixi config. A grant in the workspace's `.pixi/config.toml` is ignored. Pass the credential as an `env=` secret so it never enters a layer. Replace the `tools` and `cook` stages of the minimal example:

```dockerfile
FROM debian:bookworm-slim AS tools
RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://github.com/greenroom-robotics/pixi/releases/download/pixi-gr@0.82.1/install.sh | sh
ENV PATH=/root/.pixi/bin:$PATH
RUN printf '[azure-options."stgrcondachannel.blob.core.windows.net".auth]\npublic = false\ngeneral = true\n' \
    > /root/.pixi/config.toml
COPY --from=chef /pixi-chef /usr/local/bin/pixi-chef

FROM tools AS cook
WORKDIR /app
COPY --from=recipe /recipe .
RUN --mount=type=secret,id=azure-sas,env=AZURE_STORAGE_SAS_TOKEN \
    pixi-chef cook -e default
```

Add one `<container> = true` line per container the workspace uses (see the Greenroom Auth page). Repeat the `--mount` on the `bake` `RUN`. The SAS token must cover every granted container (an account-level token), or use service-principal credentials instead, which work the same way with `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET` and `AZURE_TENANT_ID`.

```bash
docker build --build-arg PIXI_CHEF_VERSION=<version> --secret id=azure-sas,env=AZURE_STORAGE_SAS_TOKEN -t app .
```

#### HTTP channels

For channels that use a bearer token or basic auth, mount a rattler authentication file and point `RATTLER_AUTH_FILE` at it. Replace the `cook` stage of the minimal example:

```dockerfile
FROM tools AS cook
WORKDIR /app
COPY --from=recipe /recipe .
RUN --mount=type=secret,id=rattler-auth,target=/run/secrets/rattler-auth.json \
    RATTLER_AUTH_FILE=/run/secrets/rattler-auth.json \
    pixi-chef cook -e default
```

```bash
docker build --build-arg PIXI_CHEF_VERSION=<version> --secret id=rattler-auth,src=auth.json -t app .
```

Repeat the mount on the `bake` `RUN`.

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
docker build --build-arg PIXI_CHEF_VERSION=<version> --target dev -t app-dev .
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

When several manifests under `variants/` have the same dependencies, `--into` gives their recipes the same layout, so the `cook` layer is shared by the variants whose manifest, lock and recipe files are all byte-identical. This needs every path in the manifest and lock to resolve the same from `variants/shared` as from `variants/<name>`, for example `../../common/hello`.

```dockerfile
# syntax=docker/dockerfile:1
ARG PIXI_CHEF_VERSION
ARG VARIANT=a
FROM ghcr.io/greenroom-robotics/pixi-chef:${PIXI_CHEF_VERSION} AS chef
FROM ghcr.io/prefix-dev/pixi:0.82.0 AS tools
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
