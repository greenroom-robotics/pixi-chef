import os
import re
import shutil
import subprocess
import tempfile
import unittest
import uuid
from dataclasses import dataclass
from enum import Enum
from pathlib import Path

HERE = Path(__file__).resolve().parent
FIXTURES = HERE.parent / "fixtures" / "layer_cache"
PIXI_CHEF = Path(os.environ["PIXI_CHEF_BIN"]).resolve()

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
    prepare_args: str = ""
    workspace_dir: str = "."

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

    def step_ids(self, command: str) -> list[str]:
        return [
            step for step, run in STEP_HEADER.findall(self.text) if command in run
        ]

    def cached(self, command: str) -> bool:
        ids = self.step_ids(command)
        assert len(ids) == 1, f"expected one `{command}` step, found {ids}"
        return ids[0] in CACHED.findall(self.text)


def run(args: list[str | Path], **kwargs) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args, check=True, text=True, capture_output=True, **kwargs
    )


class LayerCache:
    def __init__(self, scenario: Scenario, root: Path) -> None:
        self.scenario = scenario
        self.root = root
        self.seed = uuid.uuid4().hex
        self.tag = f"pixi-chef-test:{self.seed}"
        shutil.copytree(FIXTURES / scenario.fixture, root)
        self.chef_context = root.parent / "chef"
        self.chef_context.mkdir()
        shutil.copy(PIXI_CHEF, self.chef_context / "pixi-chef")

    def build(self) -> BuildLog:
        s = self.scenario
        result = subprocess.run(
            [
                "docker", "buildx", "build", "--progress=plain", "--load",
                "--tag", self.tag,
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
        )
        log = result.stdout + result.stderr
        if result.returncode != 0:
            raise AssertionError(log)
        return BuildLog(log)

    def run_image(self) -> str:
        return run(
            ["docker", "run", "--rm", self.tag, *self.scenario.run_command]
        ).stdout

    def edit_source(self) -> None:
        init = next(self.root.rglob("hello/hello/__init__.py"))
        init.write_text('def main():\n    print("hello v2")\n')

    def change_lock(self) -> None:
        manifest = self.root / self.scenario.manifest
        text = manifest.read_text()
        manifest.write_text(text.replace("[dependencies]\n", '[dependencies]\njq = "*"\n', 1))
        run(["pixi", "lock", "--manifest-path", manifest])

    def remove_image(self) -> None:
        subprocess.run(["docker", "rmi", "--force", self.tag], capture_output=True)


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
        prepare_args="--into variants/shared",
        workspace_dir="variants/shared",
    )


if __name__ == "__main__":
    unittest.main()
