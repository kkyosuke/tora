"""Release automation contracts: local Git remotes and a fake GitHub CLI only."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
try:
    spec = importlib.util.spec_from_file_location("detect_release", ROOT / "scripts/detect-release.py")
    detect = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(detect)
finally:
    sys.path.pop(0)


def manifest(version):
    return f'[package]\nname = "tora"\nversion = "{version}"\n'


def lock(version):
    return f'version = 4\n[[package]]\nname = "tora"\nversion = "{version}"\n'


class AutoReleaseTests(unittest.TestCase):
    def test_detects_only_increases_and_validates_lock(self):
        self.assertIsNone(detect.release_tag(manifest("0.1.0"), manifest("0.1.0"), lock("0.1.0")))
        self.assertEqual(detect.release_tag(manifest("0.9.0"), manifest("0.10.0"), lock("0.10.0")), "v0.10.0")
        for new, locked in [("0.0.9", "0.0.9"), ("0.1.1", "0.1.0"), ("1.0.0-rc1", "1.0.0-rc1")]:
            with self.assertRaises(ValueError):
                detect.release_tag(manifest("0.1.0"), manifest(new), lock(locked))

    def setUp(self):
        cache = ROOT / ".cache"
        cache.mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=cache)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.env = dict(os.environ, HOME=str(self.root), GIT_CONFIG_NOSYSTEM="1",
                        GIT_CONFIG_GLOBAL=os.devnull,
                        RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                        CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")), RELEASE_TAG="v0.1.1", GH_REPO="example/tora")
        self.git("init")
        self.git("config", "user.name", "Test")
        self.git("config", "user.email", "test@example.invalid")
        (self.repo / "src").mkdir()
        (self.repo / "src/main.rs").write_text("fn main() {}\n")
        (self.repo / "scripts").mkdir()
        for name in ("check-release.py", "release_version.py"):
            shutil.copy2(ROOT / "scripts" / name, self.repo / "scripts" / name)
        (self.repo / "Cargo.toml").write_text(manifest("0.1.0"))
        (self.repo / "Cargo.lock").write_text(lock("0.1.0"))
        self.git("add", ".")
        self.git("commit", "-m", "initial")
        self.before = self.git("rev-parse", "HEAD").stdout.strip()
        (self.repo / "Cargo.toml").write_text(manifest("0.1.1"))
        (self.repo / "Cargo.lock").write_text(lock("0.1.1"))
        self.git("commit", "-am", "bump")
        self.head = self.git("rev-parse", "HEAD").stdout.strip()
        subprocess.run(["git", "init", "--bare", str(self.root / "remote")], env=self.env, check=True, capture_output=True)
        self.git("remote", "add", "origin", str(self.root / "remote"))

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.repo, env=self.env, text=True, capture_output=True, check=True)

    def tag(self):
        return subprocess.run(["bash", str(ROOT / "scripts/tag-release.sh")], cwd=self.repo,
                              env=self.env, text=True, capture_output=True)

    def test_creates_tag_and_retry_preserves_it(self):
        for _ in range(2):
            result = self.tag()
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(self.git("rev-parse", "v0.1.1").stdout.strip(), self.head)
        self.assertIn(self.head, self.git("ls-remote", "origin", "refs/tags/v0.1.1").stdout)

    def test_tag_mismatch_cannot_overwrite_existing_tag(self):
        self.git("tag", "v0.1.1", self.before)
        result = self.tag()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refusing to overwrite", result.stderr)
        self.assertEqual(self.git("rev-parse", "v0.1.1").stdout.strip(), self.before)
        self.assertEqual(self.git("ls-remote", "origin", "refs/tags/v0.1.1").stdout, "")

    def test_detect_compares_full_push_not_just_parent(self):
        (self.repo / "README.md").write_text("next commit in same push")
        self.git("add", "README.md")
        self.git("commit", "-m", "docs")
        output = self.root / "output"
        result = subprocess.run([sys.executable, str(ROOT / "scripts/detect-release.py")], cwd=self.repo,
                                env=dict(self.env, BEFORE_SHA=self.before, GITHUB_OUTPUT=str(output)),
                                text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output.read_text(), "changed=true\ntag=v0.1.1\n")

    def test_published_assets_are_immutable_and_drafts_resume(self):
        tools = self.root / "tools"
        tools.mkdir()
        gh = tools / "gh"
        gh.write_text('''#!/usr/bin/env python3
import json, os, sys
with open(os.environ["CALLS"], "a") as f:
    f.write(json.dumps(sys.argv[1:]) + "\\n")
if sys.argv[1:3] == ["release", "view"]:
    if os.environ["STATE"] == "missing": sys.exit(1)
    print(os.environ["STATE"])
''')
        gh.chmod(0o755)
        (self.repo / "dist").mkdir()
        (self.repo / "dist/asset").write_text("fixture")
        calls = self.root / "calls"
        for state in ("false", "true", "missing"):
            calls.write_text("")
            result = subprocess.run(["bash", str(ROOT / "scripts/publish-release.sh")], cwd=self.repo,
                                    env=dict(self.env, PATH=f"{tools}:{self.env['PATH']}", CALLS=str(calls), STATE=state),
                                    text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            commands = [json.loads(line)[:2] for line in calls.read_text().splitlines()]
            if state == "false":
                self.assertEqual(commands, [["release", "view"]])
            else:
                self.assertIn(["release", "upload"], commands)
                self.assertEqual(commands[-1], ["release", "edit"])
                self.assertEqual(["release", "create"] in commands, state == "missing")
