"""Exercise completion installation prompts without touching the user's home."""
import errno
import os
from pathlib import Path
import pty
import re
import select
import shutil
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


class ProfileCompletionTerminalTests(unittest.TestCase):
    """Press Tab in real interactive shells that loaded the generated script."""

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
        (self.bin / "tora").symlink_to(BINARY)
        aws = self.bin / "aws"
        aws.write_text('#!/bin/sh\n[ "$*" = "configure list-profiles" ] || exit 2\n'
                       'printf "default\\ndev-admin\\nprod\\n"\n')
        aws.chmod(0o755)
        self.work = self.root / "work"
        self.work.mkdir()

    def complete(self, shell, typed):
        """Type `typed`, press Tab, and return the words the shell ends up with."""
        path = shutil.which(shell)
        if path is None:
            self.skipTest(f"{shell} is not installed")
        if shell == "zsh":
            command = [path, "-f", "-i"]
            setup = "autoload -Uz compinit && compinit -u -d ../zcompdump; "
        else:
            command = [path, "--noprofile", "--norc", "-i"]
            setup = ""
        env = {"HOME": str(self.root), "ZDOTDIR": str(self.root), "TERM": "dumb",
               "PATH": f"{self.bin}:/usr/bin:/bin", "PS1": "$ "}
        master, slave = pty.openpty()
        process = subprocess.Popen(command, env=env, cwd=self.work,
                                   stdin=slave, stdout=slave, stderr=slave)
        os.close(slave)
        output = bytearray()

        def wait_for(pattern):
            deadline = time.monotonic() + 10
            while True:
                match = re.search(pattern, output)
                if match:
                    return match
                if time.monotonic() > deadline:
                    self.fail(f"{pattern!r} not in {output.decode(errors='replace')!r}")
                if not select.select([master], [], [], 0.1)[0]:
                    continue
                try:
                    data = os.read(master, 8192)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    data = b""
                if not data:
                    self.fail(f"{shell} exited: {output.decode(errors='replace')!r}")
                output.extend(data)

        try:
            os.write(master, f'{setup}eval "$(tora completion {shell})"; echo "LOADED:$?"\r'.encode())
            wait_for(rb"LOADED:0")
            # Ctrl-A turns the completed line into a harmless printf. The echoed
            # format contains "%", so only printf's own output matches.
            os.write(master, typed.encode() + b"\t\x01printf '<%s>' \r")
            words = wait_for(rb"\n((?:<[^<>%]*>)+)").group(1).decode()
            return words[1:-1].split("><")
        finally:
            process.kill()
            process.wait()
            os.close(master)

    def test_tab_completes_profile_names(self):
        for shell in ("bash", "zsh"):
            for command in ("sso", "exec", "sso --no-browser"):
                with self.subTest(shell=shell, command=command):
                    words = self.complete(shell, f"tora aws {command} dev")
                    self.assertEqual(words[-1], "dev-admin", words)

    def test_command_after_separator_is_not_a_profile(self):
        for shell in ("bash", "zsh"):
            with self.subTest(shell=shell):
                words = self.complete(shell, "tora aws exec prod -- dev")
                self.assertEqual(words[-1], "dev", words)
