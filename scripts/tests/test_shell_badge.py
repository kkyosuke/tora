"""Real shell startup and prompt rendering, with AWS fully stubbed out."""
import errno
import os
from pathlib import Path
import pty
import select
import shutil
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / "target/debug/tora"


class ShellBadgeTests(unittest.TestCase):
    def check_shell(self, shell_name):
        shell = shutil.which(shell_name)
        if not shell or not BINARY.exists():
            self.skipTest(f"{shell_name} and cargo build are required")
        cache = ROOT / ".cache"
        cache.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=cache) as directory:
            root = Path(directory)
            aws = root / "aws"
            aws.write_text('#!/bin/sh\nexit 0\n')
            aws.chmod(0o755)
            config = root / "custom-config"
            config.mkdir()
            zshenv = 'export USER_ENV_LOADED=yes\n'
            zshrc = 'PROMPT="original> "\nprecmd() { PROMPT="dynamic> "; }\n'
            bashrc = 'PS1="original> "\nPROMPT_COMMAND=\'PS1="dynamic> "\'\n'
            (config / ".zshenv").write_text(zshenv)
            (config / ".zshrc").write_text(zshrc)
            (root / ".bashrc").write_text(bashrc)
            profile = "dev'$(touch injected)%F{red}`touch injected`"
            env = dict(os.environ, HOME=str(root), ZDOTDIR=str(config),
                       SHELL=shell, TMPDIR=str(root), PATH=f"{root}:{os.environ['PATH']}",
                       TERM="xterm-256color")
            master, slave = pty.openpty()
            try:
                process = subprocess.Popen([BINARY, "aws", "sso", profile], env=env,
                                           cwd=root, stdin=slave, stdout=slave, stderr=slave)
                os.close(slave)
                output = bytearray()
                sent = False
                deadline = time.monotonic() + 10
                try:
                    while time.monotonic() < deadline:
                        if select.select([master], [], [], 0.1)[0]:
                            try:
                                data = os.read(master, 8192)
                            except OSError as error:
                                if error.errno == errno.EIO:
                                    break
                                raise
                            if not data:
                                break
                            output.extend(data)
                            if not sent and b"dynamic> " in output:
                                os.write(master, b'printf "ENV=%s CONFIG=%s\\n" "$USER_ENV_LOADED" "$ZDOTDIR"\nexit\n')
                                sent = True
                        if process.poll() is not None and not select.select([master], [], [], 0)[0]:
                            break
                    status = process.wait(timeout=2)
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait()
                text = output.decode(errors="replace")
                self.assertEqual(status, 0, text)
                self.assertTrue(sent, text)
                self.assertGreaterEqual(text.count(f"[AWS: {profile}]"), 2, text)
                self.assertFalse((root / "injected").exists(), text)
                self.assertEqual(list(root.glob("tora-shell-*")), [], text)
                self.assertEqual((root / ".bashrc").read_text(), bashrc)
                self.assertEqual((config / ".zshrc").read_text(), zshrc)
                self.assertEqual((config / ".zshenv").read_text(), zshenv)
                if shell_name == "zsh":
                    self.assertIn(f"ENV=yes CONFIG={config}", text)
            finally:
                os.close(master)

    def test_zsh_preserves_config_and_adds_safe_badge(self):
        self.check_shell("zsh")

    def test_bash_preserves_config_and_adds_safe_badge(self):
        self.check_shell("bash")
