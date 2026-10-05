"""Exercise dialoguer with a real PTY, fake AWS CLI and fake child shell."""
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


class AwsTerminalTests(unittest.TestCase):
    def run_terminal(self, keys, valid=False):
        if not BINARY.exists():
            self.skipTest("cargo build is required")
        cache = ROOT / ".cache"
        cache.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=cache) as directory:
            root = Path(directory)
            aws = root / "aws"
            aws.write_text("""#!/bin/sh
if [ "$1" = configure ]; then
  printf 'default\\ndev\\n'
elif [ "$1" = sts ]; then
  if [ "$CHECK_VALID" = yes ]; then exit 0; fi
  printf 'Error loading SSO Token: Token for session does not exist\\n' >&2
  exit 1
else
  printf 'LOGIN:%s\\n' "$AWS_PROFILE"
fi
""")
            aws.chmod(0o755)
            shell = root / "shell"
            shell.write_text('#!/bin/sh\nprintf "AUTHENTICATED-SHELL:%s\\n" "$AWS_PROFILE"\n')
            shell.chmod(0o755)
            env = dict(os.environ, PATH=str(root), SHELL=str(shell), TERM="xterm-256color",
                       CHECK_VALID="yes" if valid else "no")
            env.pop("AWS_PROFILE", None)
            env.pop("AWS_DEFAULT_PROFILE", None)
            master, slave = pty.openpty()
            try:
                process = subprocess.Popen([BINARY, "aws", "sso"], env=env,
                                           stdin=slave, stdout=slave, stderr=slave)
                os.close(slave)
                slave = None
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
                            if not sent and b"Type a custom profile" in output:
                                os.write(master, keys)
                                sent = True
                        if process.poll() is not None and not select.select([master], [], [], 0)[0]:
                            break
                    self.assertTrue(sent, output.decode(errors="replace"))
                    result = process.wait(timeout=2)
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait()
                return result, output.decode(errors="replace")
            finally:
                os.close(master)
                if slave is not None:
                    os.close(slave)

    def test_arrow_selection_opens_authenticated_shell(self):
        status, output = self.run_terminal(b"\x1b[B\r")
        self.assertEqual(status, 0, output)
        self.assertIn("LOGIN:dev", output)
        self.assertIn("AUTHENTICATED-SHELL:dev", output)

    def test_valid_credentials_skip_login_after_selection(self):
        status, output = self.run_terminal(b"\x1b[B\r", valid=True)
        self.assertEqual(status, 0, output)
        self.assertNotIn("LOGIN:", output)
        self.assertIn("AUTHENTICATED-SHELL:dev", output)

    def test_cancel_does_not_login_or_start_shell(self):
        status, output = self.run_terminal(b"q")
        self.assertNotEqual(status, 0, output)
        self.assertNotIn("LOGIN:", output)
        self.assertNotIn("AUTHENTICATED-SHELL:", output)
