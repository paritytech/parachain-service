#!/usr/bin/env python3
"""Check that indexed submodule commits are available on GitHub."""

from pathlib import Path
import subprocess
import sys
from urllib.parse import urlparse
from urllib.request import Request, urlopen


def git(path, *args):
    return subprocess.run(
        ["git", "-C", str(path), *args],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    ).stdout


def check(url, commit):
    parsed = urlparse(url.replace("git@github.com:", "https://github.com/"))
    if parsed.hostname != "github.com":
        raise ValueError(f"not a GitHub repository: {url}")
    repo = parsed.path.strip("/").removesuffix(".git")
    request = Request(
        f"https://api.github.com/repos/{repo}/git/commits/{commit}",
        headers={"Accept": "application/vnd.github+json", "User-Agent": "submodule-publication-check"},
    )
    with urlopen(request, timeout=30) as response:
        if response.status != 200:
            raise ValueError(f"GitHub returned HTTP {response.status}")


def main():
    root = Path(git(Path.cwd(), "rev-parse", "--show-toplevel").strip())
    urls = {}
    paths = git(root, "config", "--blob", ":.gitmodules", "--null",
                "--get-regexp", r"^submodule\..*\.path$")
    for entry in paths.rstrip("\0").split("\0"):
        key, name = entry.split("\n", 1)
        urls[name] = git(root, "config", "--blob", ":.gitmodules", "--get",
                         key.removesuffix(".path") + ".url").strip()
    failed = False
    for entry in git(root, "ls-files", "--stage", "-z").split("\0"):
        if not entry:
            continue
        metadata, name = entry.split("\t", 1)
        mode, commit, stage = metadata.split()
        if mode != "160000":
            continue
        try:
            if stage != "0":
                raise ValueError("unresolved submodule merge conflict")
            if name not in urls:
                raise ValueError("missing submodule URL in indexed .gitmodules")
            check(urls[name], commit)
            print(f"OK {name} {commit[:12]}", flush=True)
        except (ValueError, OSError, subprocess.CalledProcessError) as error:
            detail = error.stderr.strip() if isinstance(error, subprocess.CalledProcessError) else str(error)
            print(f"FAIL {name} {commit[:12]}: {detail}", file=sys.stderr, flush=True)
            failed = True
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
