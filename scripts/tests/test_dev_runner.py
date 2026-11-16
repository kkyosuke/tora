"""Cargo runtime isolation, using real tora with a separate mock repository."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / "target/debug/tora"


class DevRunnerTests(unittest.TestCase):
    def setUp(self):
        if not BINARY.exists():
            self.skipTest("cargo build is required")
        cache = ROOT / ".cache"
        cache.mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=cache)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo with spaces"
        (self.repo / "scripts").mkdir(parents=True)
        self.runner = self.repo / "scripts/run-dev.sh"
        shutil.copy2(ROOT / "scripts/run-dev.sh", self.runner)
        self.production = self.root / "production"
        for name, content in {
            "tora/config": "aws.profile=production\naws.region=us-east-1\n",
            "tora/aws-profile-history": "production\n",
            "home/.bashrc": "# production bash\n",
            "home/.bash_profile": "# production login\n",
            "zsh/.zshrc": "# production zsh\n",
            "home/.aws/config": "[profile production]\nregion=us-east-1\n",
            "home/.aws/credentials": "production credentials\n",
            "home/.aws/sso/cache/token": "production session\n",
        }.items():
            path = self.production / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        self.before = self.snapshot()
        self.env = dict(os.environ, HOME=str(self.production / "home"),
                        TORA_HOME=str(self.production / "tora"),
                        ZDOTDIR=str(self.production / "zsh"),
                        AWS_PROFILE="production", AWS_DEFAULT_REGION="us-east-1",
                        AWS_ACCESS_KEY_ID="production-key", AWS_SECRET_ACCESS_KEY="production-secret",
                        AWS_CONFIG_FILE=str(self.production / "home/.aws/config"),
                        AWS_SHARED_CREDENTIALS_FILE=str(self.production / "home/.aws/credentials"),
                        AWS_WEB_IDENTITY_TOKEN_FILE=str(self.production / "home/.aws/sso/cache/token"),
                        AWS_CONTAINER_CREDENTIALS_RELATIVE_URI="/production", SHELL="/bin/bash")
        self.env.pop("BASH_ENV", None)
        self.dev = self.repo / ".cache/dev"

    def snapshot(self):
        return {str(p.relative_to(self.production)): p.read_bytes()
                for p in self.production.rglob("*") if p.is_file()}

    def run_dev(self, *args):
        result = subprocess.run([self.runner, BINARY, *args], env=self.env,
                                cwd=self.repo, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.snapshot(), self.before)
        return result

    def test_config_and_history_do_not_use_installer_exported_home(self):
        self.assertIn("No defaults configured", self.run_dev("config").stdout)
        self.run_dev("config", "set", "aws.profile", "development")
        self.assertEqual(self.run_dev("config", "get", "aws.profile").stdout, "development\n")
        self.run_dev("aws", "exec", "development", "--", "/bin/sh", "-c", "exit 0")
        self.assertEqual((self.dev / "tora/config").read_text(), "aws.profile=development\n")
        self.assertEqual((self.dev / "tora/aws-profile-history").read_text(), "development\n")

    def test_completion_only_modifies_development_startup_files(self):
        for shell, rc in [("bash", ".bashrc"), ("zsh", ".zshrc")]:
            self.run_dev("completion", "install", "--shell", shell, "--yes")
            self.assertIn("# >>> tora completion >>>", (self.dev / "home" / rc).read_text())
            self.assertTrue((self.dev / "tora/completions" / shell).is_file())

    def test_aws_logout_only_sees_development_cache_and_configuration(self):
        tools = self.root / "tools"
        tools.mkdir()
        aws = tools / "aws"
        aws.write_text('''#!/bin/sh
[ "$1 $2" = 'sso logout' ] || exit 2
[ -z "$AWS_ACCESS_KEY_ID$AWS_SECRET_ACCESS_KEY$AWS_WEB_IDENTITY_TOKEN_FILE$AWS_PROFILE$AWS_CONTAINER_CREDENTIALS_RELATIVE_URI" ] || exit 3
[ "$AWS_EC2_METADATA_DISABLED" = true ] || exit 4
[ "$AWS_CONFIG_FILE" = "$HOME/.aws/config" ] || exit 5
[ "$AWS_SHARED_CREDENTIALS_FILE" = "$HOME/.aws/credentials" ] || exit 6
rm -f "$HOME/.aws/sso/cache/token"
''')
        aws.chmod(0o755)
        self.env["PATH"] = f"{tools}:{self.env['PATH']}"
        token = self.dev / "home/.aws/sso/cache/token"
        token.parent.mkdir(parents=True)
        token.write_text("dev session")
        self.run_dev("aws", "logout")
        self.assertFalse(token.exists())

    def test_child_tora_resolves_to_development_binary(self):
        fake_bin = self.production / "bin"
        fake_bin.mkdir()
        fake = fake_bin / "tora"
        fake.write_text("#!/bin/sh\necho production-binary\nexit 9\n")
        fake.chmod(0o755)
        self.before = self.snapshot()
        self.env["PATH"] = f"{fake_bin}:{self.env['PATH']}"
        result = self.run_dev("aws", "exec", "dev", "--", "tora", "config")
        self.assertIn(str(self.dev / "tora/config"), result.stdout)
        self.assertNotIn("production-binary", result.stdout)

    def test_update_is_rejected_and_exit_status_is_preserved(self):
        result = subprocess.run([self.runner, BINARY, "update"], env=self.env,
                                cwd=self.repo, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("self-update requires", result.stderr)
        self.assertEqual(self.snapshot(), self.before)
        self.assertFalse((self.dev / "tora/bin/tora").exists())
        result = subprocess.run([self.runner, "/bin/sh", "-c", "exit 17"], env=self.env)
        self.assertEqual(result.returncode, 17)
