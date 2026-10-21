# pixi-chef

Splits a pixi workspace's container build into a dependency layer that only changes with the lock, and a source layer that changes with every edit.

## Language

**Source package**:
A lock entry built from source (a local directory, git or URL) rather than fetched as a built artifact.
_Avoid_: Path dep, path package, local package

**Binary package**:
A lock entry installed from a built artifact, wherever that artifact lives, including a `.conda` file on local disk.
_Avoid_: Channel package, remote package

**Recipe**:
The trimmed copy of a repo holding only the workspace manifest, its lock, and the metadata files of each source package, laid out at their real paths.
_Avoid_: Skeleton, metadata tree, manifests

**Into directory**:
The directory `prepare --into` places the manifest and lock at. Cook and bake both run there, so `PIXI_PROJECT_ROOT` is that directory at runtime, and every path the manifest and lock name must resolve to the same repo path from it as from the manifest's own directory.
_Avoid_: Recipe root, workspace dir

**Export**:
A tree laid out as a filesystem root that holds one installed environment, every local source package's directory, the activation hook and the entrypoint, ready to copy onto a runtime image.
_Avoid_: Bundle, pack, output
