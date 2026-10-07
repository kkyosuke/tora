"""Exercise completion installation prompts without touching the user's home."""
import errno
import os
from pathlib import Path
import pty
import select
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / "target/debug/tora"


class CompletionTerminalTests(unittest.TestCase):
    def setUp(self):
        if not BINARY.exists():
            self.skipTest("cargo build is required")
        cache = ROOT / ".cache"
        cache.mkdir(exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(dir=cache)
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        # Model ps's two calls without depending on the test runner's shell.
        ps = self.bin / "ps"
        ps.write_text('#!/bin/sh\ncase "$4" in ppid=) echo 123;; comm=) echo "$TEST_PARENT_SHELL";; esac\n')
        ps.chmod(0o755)

    def run_terminal(self, steps, parent="/bin/zsh", shell="/bin/bash", args=()):
        env = dict(os.environ, HOME=str(self.root), PATH=str(self.bin),
                   SHELL=shell, TEST_PARENT_SHELL=parent, TERM="xterm-256color")
        env.pop("TORA_HOME", None)
        env.pop("ZDOTDIR", None)
        master, slave = pty.openpty()
        process = None
        try:
            process = subprocess.Popen([BINARY, "completion", "install", *args],
                                       env=env, cwd=self.root,
                                       stdin=slave, stdout=slave, stderr=slave)
            os.close(slave)
            slave = None
            output = bytearray()
            step = 0
            deadline = time.monotonic() + 10
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
                    if step < len(steps) and steps[step][0] in output:
                        os.write(master, steps[step][1])
                        step += 1
                if process.poll() is not None and not select.select([master], [], [], 0)[0]:
                    break
            self.assertEqual(step, len(steps), output.decode(errors="replace"))
            return process.wait(timeout=2), output.decode(errors="replace")
        finally:
            if process is not None and process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
            if slave is not None:
                os.close(slave)

    def test_parent_shell_wins_and_declining_writes_nothing(self):
        status, output = self.run_terminal([
            (b"Completion loader file", b"\r"),
            (b"Install completion?", b"n\r"),
        ])
        self.assertEqual(status, 0, output)
        self.assertIn("Shell: zsh", output)
        self.assertFalse((self.root / ".tora").exists())
        self.assertFalse((self.root / ".zshrc").exists())

    def test_custom_destination_and_repeat_without_prompts(self):
        status, output = self.run_terminal([
            (b"Completion loader file", b"custom/zsh\r"),
            (b"Install completion?", b"y\r"),
        ])
        self.assertEqual(status, 0, output)
        self.assertTrue((self.root / "custom/zsh").is_file())
        original = (self.root / ".zshrc").read_text()
        status, output = self.run_terminal([])
        self.assertEqual(status, 0, output)
        self.assertIn("already installed", output)
        self.assertEqual((self.root / ".zshrc").read_text(), original)

    def test_unknown_parent_falls_back_to_shell_environment(self):
        status, output = self.run_terminal([
            (b"Completion loader file", b"\r"),
            (b"Install completion?", b"\r"),
        ], parent="python3")
        self.assertEqual(status, 0, output)
        self.assertIn("Shell: bash", output)
        self.assertFalse((self.root / ".bashrc").exists())

    def test_unknown_shell_can_cancel_selection(self):
        status, output = self.run_terminal([(b"Esc/q to cancel", b"q")], parent="python3", shell="/bin/fish")
        self.assertEqual(status, 0, output)
        self.assertFalse((self.root / ".tora").exists())

    def test_explicit_shell_overrides_parent(self):
        status, output = self.run_terminal([
            (b"Completion loader file", b"\r"),
            (b"Install completion?", b"n\r"),
        ], args=("--shell", "bash"))
        self.assertEqual(status, 0, output)
        self.assertIn("Shell: bash", output)
