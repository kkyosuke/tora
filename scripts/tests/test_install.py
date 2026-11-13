"""Offline installer contract tests. All effects stay inside the worktree."""
import hashlib
import io
import os
import shutil
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class InstallerTests(unittest.TestCase):
    def setUp(self):
        cache = ROOT / ".cache"
        cache.mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=cache)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "tools"
        self.bin.mkdir()
        self.home = self.root / "home with spaces"
        self.fixture = self.root / "release"
        self.fixture.mkdir()
        self.asset = "tora-linux-amd64.tar.gz"
        self.env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}",
                        TORA_HOME=str(self.home), FIXTURE=str(self.fixture),
                        HOME=str(self.root / "shell-home"), SHELL="/bin/bash", LC_ALL="C")
        self.env.pop("ZDOTDIR", None)
        self.env.pop("TORA_VERSION", None)
        self.tool("uname", '#!/bin/sh\ncase "$1" in -s) echo Linux;; -m) echo x86_64;; esac\n')
        self.tool("curl", '''#!/usr/bin/env python3
import os, sys, shutil
from pathlib import Path
args = sys.argv[1:]
url = next(a for a in args if a.startswith("https://"))
if url.endswith("/latest"):
    print("https://github.com/KKyosuke/tora/releases/tag/v0.1.0", end="")
else:
    assert "/download/v0.1.0/" in url, url
    shutil.copyfile(Path(os.environ["FIXTURE"]) / url.rsplit("/", 1)[1], args[args.index("-o") + 1])
''')
        self.archive()

    def tool(self, name, body):
        path = self.bin / name
        path.write_text(body)
        path.chmod(0o755)

    def archive(self, entry="tora", version="0.1.0", symlink=False, extra=False):
        path = self.fixture / self.asset
        with tarfile.open(path, "w:gz") as archive:
            data = f'#!/bin/sh\necho "tora {version}"\n'.encode()
            info = tarfile.TarInfo(entry)
            info.mode = 0o755
            if symlink:
                info.type = tarfile.SYMTYPE
                info.linkname = "/bin/sh"
                archive.addfile(info)
            else:
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))
            if extra:
                archive.addfile(tarfile.TarInfo("extra"))
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        (self.fixture / (self.asset + ".sha256")).write_text(f"{digest}  {self.asset}\n")
        (self.fixture / (self.asset + ".version")).write_text("v0.1.0\n")

    def run_install(self, *args):
        return subprocess.run(["bash", str(ROOT / "scripts/install.sh"), *args],
                              env=self.env, text=True, capture_output=True)

    def installed(self):
        return self.home / "bin/tora"

    def old_binary(self):
        self.installed().parent.mkdir(parents=True, exist_ok=True)
        self.installed().write_text("previous binary")

    def assert_preserved(self, result):
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.installed().read_text(), "previous binary")
        self.assertFalse((self.home / "update.lock").exists())
        self.assertEqual(list(self.installed().parent.glob(".update.*")), [])

    def test_latest_install_and_explicit_reinstall(self):
        for args in [(), ("--version", "0.1.0"), ("--version", "v0.1.0")]:
            result = self.run_install(*args)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(subprocess.check_output([self.installed(), "--version"], text=True), "tora 0.1.0\n")
            self.assertFalse((self.home / "update.lock").exists())

    def test_bash_path_is_persistent_and_idempotent(self):
        for _ in range(2):
            self.assertEqual(self.run_install().returncode, 0)
        for name in (".bashrc", ".bash_profile"):
            path = Path(self.env["HOME"]) / name
            self.assertEqual(path.read_text().count("# >>> tora PATH >>>"), 1)
            result = subprocess.run(["bash", "--noprofile", "--norc", "-c",
                                     'source "$1"; source "$1"; command -v tora; printf "%s" "$TORA_HOME"',
                                     "bash", str(path)], env=self.env, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, f"{self.installed()}\n{self.home}")

    def test_default_home_and_existing_path_are_persisted(self):
        self.env.pop("TORA_HOME")
        self.home = Path(self.env["HOME"]) / ".tora"
        self.env["PATH"] = f"{self.home / 'bin'}:{self.env['PATH']}"
        result = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.installed().exists())
        self.assertIn("# >>> tora PATH >>>", (Path(self.env["HOME"]) / ".bashrc").read_text())

    def test_zsh_uses_zdotdir_and_preserves_existing_content(self):
        self.env["SHELL"] = "/bin/zsh"
        directory = self.root / "zsh config"
        directory.mkdir()
        self.env["ZDOTDIR"] = str(directory)
        path = directory / ".zshrc"
        path.write_text("# existing config\nexport KEEP_ME=yes\n")
        self.assertEqual(self.run_install().returncode, 0)
        self.assertTrue(path.read_text().startswith("# existing config\nexport KEEP_ME=yes\n"))
        self.assertFalse((Path(self.env["HOME"]) / ".bashrc").exists())
        if shutil.which("zsh"):
            result = subprocess.run(["zsh", "-f", "-c", 'source "$1"; command -v tora', "zsh", str(path)],
                                    env=self.env, capture_output=True, text=True)
            self.assertEqual(result.stdout.strip(), str(self.installed()))

    def test_custom_path_is_quoted_and_replaced_on_reinstall(self):
        self.assertEqual(self.run_install().returncode, 0)
        self.home = self.root / "custom '$(touch injected) `touch injected`"
        self.env["TORA_HOME"] = str(self.home)
        result = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        path = Path(self.env["HOME"]) / ".bashrc"
        self.assertEqual(path.read_text().count("# >>> tora PATH >>>"), 1)
        result = subprocess.run(["bash", "--noprofile", "--norc", "-c", 'source "$1"; command -v tora',
                                 "bash", str(path)], env=self.env, cwd=self.root, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), str(self.installed()))
        self.assertFalse((self.root / "injected").exists())

    def test_bash_keeps_login_choice_and_dotfile_symlink(self):
        home = Path(self.env["HOME"])
        home.mkdir()
        target = self.root / "dotfile"
        target.write_text("# preserved\n")
        (home / ".bashrc").symlink_to(target)
        (home / ".profile").write_text("# login\n")
        self.assertEqual(self.run_install().returncode, 0)
        self.assertTrue((home / ".bashrc").is_symlink())
        self.assertTrue(target.read_text().startswith("# preserved\n"))
        self.assertIn("# >>> tora PATH >>>", (home / ".profile").read_text())
        self.assertFalse((home / ".bash_profile").exists())

    def test_failed_verification_does_not_modify_shell_files(self):
        (self.fixture / self.asset).write_bytes(b"corrupt")
        self.assertNotEqual(self.run_install().returncode, 0)
        self.assertFalse(Path(self.env["HOME"]).exists())

    def test_damaged_markers_and_unsupported_shell_give_manual_guidance(self):
        home = Path(self.env["HOME"])
        home.mkdir()
        path = home / ".bashrc"
        original = "# >>> tora PATH >>>\nunfinished\n"
        path.write_text(original)
        result = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PATH setup needs attention", result.stderr)
        self.assertEqual(path.read_text(), original)
        self.assertFalse((home / ".bash_profile").exists())
        self.env["SHELL"] = "/bin/fish"
        result = self.run_install()
        self.assertEqual(result.returncode, 0)
        self.assertIn("supports Bash/Zsh", result.stderr)
        self.assertEqual(path.read_text(), original)
        self.env["SHELL"] = ""
        result = self.run_install()
        self.assertEqual(result.returncode, 0)
        self.assertIn("PATH setup needs attention", result.stderr)

    def test_invalid_version_and_unknown_arguments(self):
        for args in [("--version",), ("--bad",), ("--version", "v1.2.3/../../x"),
                     ("--version", "$(touch pwned)"), ("--version", "01.2.3")]:
            self.assertNotEqual(self.run_install(*args).returncode, 0)
            self.assertFalse(self.home.exists())

    def test_checksum_failure_preserves_old_binary(self):
        self.old_binary()
        (self.fixture / self.asset).write_bytes(b"corrupt")
        self.assert_preserved(self.run_install())

    def test_missing_download_preserves_old_binary(self):
        self.old_binary()
        (self.fixture / self.asset).unlink()
        self.assert_preserved(self.run_install())

    def test_unsafe_archives_preserve_old_binary(self):
        self.old_binary()
        for options in [dict(entry="../tora"), dict(symlink=True), dict(extra=True)]:
            self.archive(**options)
            self.assert_preserved(self.run_install())

    def test_wrong_binary_version_preserves_old_binary(self):
        self.old_binary()
        self.archive(version="0.2.0")
        self.assert_preserved(self.run_install())

    def test_wrong_metadata_preserves_old_binary(self):
        self.old_binary()
        (self.fixture / (self.asset + ".version")).write_text("v0.2.0\n")
        self.assert_preserved(self.run_install())

    def test_busy_lock_is_not_removed(self):
        self.old_binary()
        lock = self.home / "update.lock"
        lock.mkdir()
        result = self.run_install()
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(lock.is_dir())
        self.assertEqual(self.installed().read_text(), "previous binary")

    def test_unsupported_platform(self):
        self.tool("uname", '#!/bin/sh\necho unsupported\n')
        self.assertNotEqual(self.run_install().returncode, 0)
        self.assertFalse(self.home.exists())

    def test_embedded_installer_updates_the_installed_cli(self):
        binary = ROOT / "target/debug/tora"
        if not binary.exists():
            self.skipTest("cargo build is required for the CLI update integration test")
        self.installed().parent.mkdir(parents=True)
        shutil.copy2(binary, self.installed())
        before = self.installed().read_bytes()
        # `update` must resolve latest even with an ambient version override.
        self.env["TORA_VERSION"] = "invalid"
        result = subprocess.run([self.installed(), "update"], env=self.env,
                                text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotEqual(self.installed().read_bytes(), before)
        self.assertEqual(subprocess.check_output([self.installed(), "--version"], text=True), "tora 0.1.0\n")

    def test_source_binary_refuses_to_update_another_installation(self):
        binary = ROOT / "target/debug/tora"
        if not binary.exists():
            self.skipTest("cargo build is required for the CLI update integration test")
        self.old_binary()
        result = subprocess.run([binary, "update"], env=self.env,
                                text=True, capture_output=True)
        self.assert_preserved(result)
        self.assertIn("self-update requires the installed binary", result.stderr)


if __name__ == "__main__":
    unittest.main()
