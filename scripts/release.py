#!/usr/bin/env python3
"""Select a version bump and open a release preparation PR (requires git/gh)."""
import argparse
from pathlib import Path
import re
import subprocess
import sys

from release_version import parse_version

ROOT = Path(__file__).resolve().parents[1]
KINDS = ("major", "minor", "patch")


def run(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def bump(version, kind):
    parts = list(parse_version(version))
    index = KINDS.index(kind)
    parts[index] += 1
    parts[index + 1:] = [0] * (2 - index)
    return ".".join(map(str, parts))


def version_field(text, section, name):
    candidates = []
    offset = 0
    for block in re.split(r"(?m)(?=^\[)", text):
        if block.splitlines()[0:1] == [section] and re.search(
            rf'^name = "{re.escape(name)}"$', block, re.M
        ):
            candidates.append((block, offset))
        offset += len(block)
    if len(candidates) != 1:
        raise ValueError(f"{section} {name} を一意に取得できません")
    block, offset = candidates[0]
    matches = list(re.finditer(r'^version = "([^"]+)"$', block, re.M))
    if len(matches) != 1:
        raise ValueError(f"{section} {name} の version を一意に取得できません")
    match = matches[0]
    return match.group(1), offset + match.start(1)


def prepare(manifest, lock, kind):
    old, manifest_offset = version_field(manifest, "[package]", "tora")
    locked, lock_offset = version_field(lock, "[[package]]", "tora")
    if locked != old:
        raise ValueError("Cargo.toml と Cargo.lock のバージョンが一致しません")
    new = bump(old, kind)
    return new, (manifest[:manifest_offset] + new + manifest[manifest_offset + len(old):],
                 lock[:lock_offset] + new + lock[lock_offset + len(old):])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", nargs="?", choices=KINDS)
    args = parser.parse_args()
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
    original = [path.read_text() for path in paths]
    kind = args.kind
    if kind is None:
        if not sys.stdin.isatty():
            parser.error("引数に major / minor / patch を指定してください")
        for i, option in enumerate(KINDS, 1):
            version, _ = prepare(*original, option)
            print(f"{i}. {option}: {version}")
        choice = input("更新する種類 [1/2/3 または major/minor/patch]: ").strip()
        kind = dict(zip(("1", "2", "3"), KINDS)).get(choice, choice)
        if kind not in KINDS:
            parser.error("major / minor / patch を選択してください")
    version, updated = prepare(*original, kind)
    if run("git", "status", "--porcelain"):
        raise ValueError("未コミットの変更があります。先にコミットまたは退避してください")
    run("gh", "auth", "status")
    run("git", "fetch", "origin", "main")
    if run("git", "rev-parse", "HEAD") != run("git", "rev-parse", "FETCH_HEAD"):
        raise ValueError("最新の origin/main と同じコミットで実行してください")
    branch = f"release/v{version}"
    # Never overwrite an existing release branch or tag, including published versions.
    for ref in (f"refs/heads/{branch}", f"refs/tags/v{version}"):
        if run("git", "ls-remote", "origin", ref):
            raise ValueError(f"リモートに {ref} が既に存在します")
    if run("git", "tag", "--list", f"v{version}"):
        raise ValueError(f"タグ v{version} が既に存在します")
    run("git", "switch", "-c", branch)
    for path, content in zip(paths, updated):
        path.write_text(content)
    run("cargo", "metadata", "--locked", "--no-deps", "--format-version", "1")
    run("git", "add", "Cargo.toml", "Cargo.lock")
    run("git", "commit", "-m", f"chore: prepare v{version} release")
    run("git", "push", "-u", "origin", branch)
    body = (f"tora を v{version} に更新します（{kind}）。\n\n"
            "Cargo.toml と Cargo.lock のパッケージバージョンのみを更新します。\n"
            "CI 成功後に main へマージし、マージ後のコミットで次を実行してください。\n\n"
            f"```sh\ngit tag v{version} && git push origin v{version}\n```\n")
    print(run("gh", "pr", "create", "--base", "main", "--head", branch,
              "--title", f"chore: release v{version}", "--body", body))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, subprocess.CalledProcessError, OSError, EOFError) as error:
        sys.exit(f"リリース PR の作成を停止しました: {error}\n"
                 "途中の変更・ブランチは保持されます。git status で確認してください。")
