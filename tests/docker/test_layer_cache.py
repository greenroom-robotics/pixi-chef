import os
import re
import shutil
import subprocess
import tempfile
import unittest
import uuid
from dataclasses import dataclass
from enum import Enum
from pathlib import Path, PurePosixPath

HERE = Path(__file__).resolve().parent
FIXTURES = HERE.parent / "fixtures" / "layer_cache"
PIXI_CHEF = Path(os.environ["PIXI_CHEF_BIN"]).resolve()

SUBPROCESS_TIMEOUT = 20 * 60

STEP_HEADER = re.compile(r"^#(\d+) \[[^\]]+\] RUN (.*)$", re.MULTILINE)
CACHED = re.compile(r"^#(\d+) CACHED$", re.MULTILINE)


class Target(Enum):
    RUNTIME = "runtime"
    DEV = "dev"


@dataclass(frozen=True)
class Scenario:
    fixture: str
    manifest: str
    target: Target
    env: str = "default"
    into: PurePosixPath | None = None

    @property
    def prepare_args(self) -> str:
        return "" if self.into is None else f"--into {self.into}"

    @property
    def workspace_dir(self) -> str:
        return "." if self.into is None else str(self.into)

    @property
    def run_command(self) -> list[str]:
        match self.target:
            case Target.RUNTIME:
                return ["hello"]
            case Target.DEV:
                return ["pixi", "run", "--as-is", "-e", self.env, "hello"]


@dataclass(frozen=True)
class BuildLog:
    text: str

    def step_ids(self, command: str) -> set[str]:
        return {
            step for step, run in STEP_HEADER.findall(self.text) if command in run
        }

    def cached(self, command: str) -> bool:
        ids = self.step_ids(command)
        if len(ids) != 1:
            raise AssertionError(f"expected one `{command}` step, found {ids}")
        return ids.pop() in CACHED.findall(self.text)


def run(args: list[str | Path]) -> str:
    result = subprocess.run(
        args, text=True, capture_output=True, timeout=SUBPROCESS_TIMEOUT
    )
    if result.returncode != 0:
        raise AssertionError(
            f"{args} exited {result.returncode}\n{result.stdout}\n{result.stderr}"
        )
    return result.stdout


class LayerCache:
    def __init__(self, scenario: Scenario, root: Path) -> None:
        self.scenario = scenario
        self.root = root
        self.seed = uuid.uuid4().hex
        self.tags: list[str] = []
        shutil.copytree(FIXTURES / scenario.fixture, root)
        self.chef_context = root.parent / "chef"
        self.chef_context.mkdir()
        shutil.copy(PIXI_CHEF, self.chef_context / "pixi-chef")

    def build(self) -> BuildLog:
        s = self.scenario
        tag = f"pixi-chef-test:{uuid.uuid4().hex}"
        self.tags.append(tag)
        result = subprocess.run(
            [
                "docker", "buildx", "build", "--progress=plain", "--load",
                "--tag", tag,
                "--target", s.target.value,
                "--file", HERE / "Dockerfile",
                "--build-context", f"chef={self.chef_context}",
                "--build-context", f"harness={HERE}",
                "--build-arg", f"SEED={self.seed}",
                "--build-arg", f"ENV={s.env}",
                "--build-arg", f"MANIFEST={s.manifest}",
                "--build-arg", f"PREPARE_ARGS={s.prepare_args}",
                "--build-arg", f"WORKSPACE_DIR={s.workspace_dir}",
                self.root,
            ],
            text=True,
            capture_output=True,
            timeout=SUBPROCESS_TIMEOUT,
        )
        log = result.stdout + result.stderr
        if result.returncode != 0:
            raise AssertionError(log)
        return BuildLog(log)

    def run_image(self) -> str:
        return run(
            ["docker", "run", "--rm", self.tags[-1], *self.scenario.run_command]
        )

    def edit_source(self) -> None:
        init = next(self.root.rglob("hello/hello/__init__.py"))
        init.write_text('def main():\n    print("hello v2")\n')

    def change_lock(self) -> None:
        manifest = self.root / self.scenario.manifest
        before = manifest.read_bytes()
        run(["pixi", "update", "--no-install", "--manifest-path", manifest, "jq"])
        if manifest.read_bytes() != before:
            raise AssertionError("updating the lock changed the manifest")

    def remove_image(self) -> None:
        subprocess.run(
            ["docker", "rmi", "--force", *self.tags],
            capture_output=True,
            timeout=SUBPROCESS_TIMEOUT,
        )


COOK = "pixi-chef cook"


class LayerCacheChecks:
    scenario: Scenario

    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.cache = LayerCache(self.scenario, Path(tmp.name) / "repo")
        self.addCleanup(self.cache.remove_image)

    def test_source_edit_reuses_cook_and_lock_change_reruns_it(self) -> None:
        first = self.cache.build()
        self.assertFalse(first.cached(COOK))
        self.assertEqual(self.cache.run_image(), "hello v1\n")

        self.cache.edit_source()
        edited = self.cache.build()
        self.assertTrue(edited.cached(COOK))
        self.assertEqual(self.cache.run_image(), "hello v2\n")

        self.cache.change_lock()
        relocked = self.cache.build()
        self.assertFalse(relocked.cached(COOK))
        self.assertEqual(self.cache.run_image(), "hello v2\n")


class Plain(LayerCacheChecks, unittest.TestCase):
    scenario = Scenario("plain", "pixi.toml", Target.RUNTIME)


class Dev(LayerCacheChecks, unittest.TestCase):
    scenario = Scenario("plain", "pixi.toml", Target.DEV, env="dev")


class Into(LayerCacheChecks, unittest.TestCase):
    scenario = Scenario(
        "variants",
        "variants/a/pixi.toml",
        Target.RUNTIME,
        into=PurePosixPath("variants/shared"),
    )


if __name__ == "__main__":
    unittest.main()
