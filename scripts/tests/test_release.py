"""Release preparation contracts; no pushes or PRs are made by these tests."""
import importlib.util
from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("release", ROOT / "scripts/release.py")
release = importlib.util.module_from_spec(spec)
sys.path.insert(0, str(ROOT / "scripts"))
try:
    spec.loader.exec_module(release)
finally:
    sys.path.pop(0)

MANIFEST = '[package]\nname = "tora"\nversion = "1.2.3"\n\n[dependencies]\nfoo = "1.2.3"\n'
LOCK = 'version = 4\n\n[[package]]\nname = "foo"\nversion = "1.2.3"\n\n[[package]]\nname = "tora"\nversion = "1.2.3"\n'


class ReleaseTests(unittest.TestCase):
    def test_bumps_only_package_versions(self):
        for kind, expected in zip(release.KINDS, ("2.0.0", "1.3.0", "1.2.4")):
            version, (manifest, lock) = release.prepare(MANIFEST, LOCK, kind)
            self.assertEqual(version, expected)
            self.assertIn(f'name = "tora"\nversion = "{expected}"', manifest)
            self.assertIn(f'name = "tora"\nversion = "{expected}"', lock)
            self.assertIn('foo = "1.2.3"', manifest)
            self.assertIn('name = "foo"\nversion = "1.2.3"', lock)

    def test_rejects_invalid_versions_and_mismatched_lock(self):
        for version in ("1.2.3-beta", "01.2.3", "1.2"):
            with self.assertRaises(ValueError):
                release.bump(version, "patch")
        with self.assertRaises(ValueError):
            release.prepare(MANIFEST, LOCK.replace('name = "tora"', 'name = "other"'), "patch")
        with self.assertRaises(ValueError):
            release.prepare(MANIFEST, LOCK.replace('version = "1.2.3"', 'version = "1.2.2"'), "patch")

    def test_ambiguous_package_blocks_are_rejected_even_without_version(self):
        for extra in ('[[package]]\nname = "tora"\n',
                      '[[package]]\nname = "tora"\nversion = "1.2.3"\n'):
            with self.assertRaises(ValueError):
                release.prepare(MANIFEST, LOCK + extra, "patch")

    def exercise(self, overrides=None):
        cache = ROOT / ".cache"
        cache.mkdir(exist_ok=True)
        temp = tempfile.TemporaryDirectory(dir=cache)
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        (root / "Cargo.toml").write_text(MANIFEST)
        (root / "Cargo.lock").write_text(LOCK)
        responses = {("git", "rev-parse", "HEAD"): "abc",
                     ("git", "rev-parse", "FETCH_HEAD"): "abc"}
        responses.update(overrides or {})
        mock = patch.object(release, "run", side_effect=lambda *args: responses.get(args, ""))
        return root, mock

    def test_creates_pr_without_tagging(self):
        root, mock = self.exercise()
        with patch.object(release, "ROOT", root), mock as calls, patch("sys.argv", ["release.py", "patch"]):
            release.main()
        commands = [c.args for c in calls.call_args_list]
        self.assertIn(("git", "push", "-u", "origin", "release/v1.2.4"), commands)
        self.assertEqual(commands[-1][:3], ("gh", "pr", "create"))
        self.assertIn('version = "1.2.4"', (root / "Cargo.lock").read_text())
        self.assertEqual([c for c in commands if c[:2] == ("git", "tag")],
                         [("git", "tag", "--list", "v1.2.4")])

    def test_preconditions_stop_before_changes(self):
        for overrides in (
            {("git", "status", "--porcelain"): " M README.md"},
            {("git", "rev-parse", "FETCH_HEAD"): "new"},
            {("git", "ls-remote", "origin", "refs/heads/release/v1.2.4"): "exists"},
            {("git", "ls-remote", "origin", "refs/tags/v1.2.4"): "exists"},
        ):
            root, mock = self.exercise(overrides)
            with patch.object(release, "ROOT", root), mock as calls, patch("sys.argv", ["release.py", "patch"]):
                with self.assertRaises(ValueError):
                    release.main()
            self.assertFalse(any(c.args[:2] == ("git", "switch") for c in calls.call_args_list))
            self.assertEqual((root / "Cargo.toml").read_text(), MANIFEST)

    def test_interactive_selection(self):
        root, mock = self.exercise()
        with patch.object(release, "ROOT", root), mock as calls, \
                patch("sys.argv", ["release.py"]), \
                patch("sys.stdin.isatty", return_value=True), \
                patch("builtins.input", return_value="2"), patch("builtins.print"):
            release.main()
        self.assertIn(("git", "switch", "-c", "release/v1.3.0"),
                      [c.args for c in calls.call_args_list])

    def test_noninteractive_requires_kind(self):
        with patch("sys.argv", ["release.py"]), \
                patch("sys.stdin.isatty", return_value=False), \
                patch("sys.stderr"), patch.object(release, "run") as calls:
            with self.assertRaises(SystemExit) as error:
                release.main()
        self.assertEqual(error.exception.code, 2)
        calls.assert_not_called()
