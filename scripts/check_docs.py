#!/usr/bin/env python3
"""Documentation gates.

Default run (CI, every push):
  * sample version strings in the docs match the workspace version
  * every `server setup` flag and every `domain` subcommand is documented
  * README.md and README.id.md keep the same heading structure

With --since <ref> (release workflow, tag builds):
  * both READMEs changed since the previous release
"""

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
README = ROOT / "README.md"
README_ID = ROOT / "README.id.md"
DOC_FILES = [README, README_ID] + sorted((ROOT / "docs").glob("*.md"))
SURFACE_FILES = DOC_FILES + [ROOT / "skills" / "llms.txt"]

failures: list[str] = []


def fail(message: str) -> None:
    failures.append(message)


def workspace_version() -> str:
    text = (ROOT / "Cargo.toml").read_text()
    match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not match:
        sys.exit("Cargo.toml has no workspace version")
    return match.group(1)


def check_sample_versions(version: str) -> None:
    patterns = [
        re.compile(r'"version"\s*:\s*"(\d+\.\d+\.\d+)"'),
        re.compile(r"--version <tag>.*?\bv(\d+\.\d+\.\d+)"),
    ]
    for path in DOC_FILES:
        for lineno, line in enumerate(path.read_text().splitlines(), 1):
            for pattern in patterns:
                found = pattern.search(line)
                if found and found.group(1) != version:
                    rel = path.relative_to(ROOT)
                    fail(f"{rel}:{lineno} shows version {found.group(1)}, workspace is {version}")


def setup_flags() -> set[str]:
    text = (ROOT / "crates" / "skoci" / "src" / "main.rs").read_text()
    block = re.search(r"Setup \{(.*?)\n    \},", text, re.DOTALL)
    if not block:
        sys.exit("could not locate the `server setup` clap block in crates/skoci/src/main.rs")
    flags = set()
    for field in re.finditer(r"#\[arg\((.*?)\)\]\s*\n\s*([a-z_]+):", block.group(1), re.DOTALL):
        attrs, name = field.groups()
        if "long" not in attrs:
            continue
        explicit = re.search(r'long\s*=\s*"([^"]+)"', attrs)
        flags.add(explicit.group(1) if explicit else name.replace("_", "-"))
    return flags


def domain_subcommands() -> set[str]:
    text = (ROOT / "crates" / "skoci-cli" / "src" / "args.rs").read_text()
    block = re.search(r"pub enum DomainCommands \{(.*?)\n\}", text, re.DOTALL)
    if not block:
        sys.exit("could not locate `DomainCommands` in crates/skoci-cli/src/args.rs")
    return {name.lower() for name in re.findall(r"^\s{4}([A-Z][A-Za-z]*)\s*\{", block.group(1), re.MULTILINE)}


def check_surface_documented() -> None:
    corpus = "\n".join(path.read_text() for path in SURFACE_FILES if path.exists())
    for flag in sorted(setup_flags()):
        if not re.search(rf"--{re.escape(flag)}(?![\w-])", corpus):
            fail(f"`server setup --{flag}` is not documented in the READMEs, docs/ or skills/llms.txt")
    for sub in sorted(domain_subcommands()):
        if not re.search(rf"domain {re.escape(sub)}(?![\w-])", corpus):
            fail(f"`skoci domain {sub}` is not documented in the READMEs, docs/ or skills/llms.txt")


def headings(path: pathlib.Path) -> list[int]:
    levels = []
    in_fence = False
    for line in path.read_text().splitlines():
        if line.startswith("```"):
            in_fence = not in_fence
            continue
        if not in_fence and line.startswith("#"):
            levels.append(len(line) - len(line.lstrip("#")))
    return levels


def check_readme_parity() -> None:
    english, indonesian = headings(README), headings(README_ID)
    if english != indonesian:
        fail(
            "README.md and README.id.md heading structure diverged: "
            f"{len(english)} vs {len(indonesian)} headings, levels {english} vs {indonesian}"
        )


def check_release_touched_readmes(since: str) -> None:
    changed = subprocess.run(
        ["git", "diff", "--name-only", f"{since}..HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()
    for name in ("README.md", "README.id.md"):
        if name not in changed:
            fail(
                f"{name} was not updated since {since}. Every release documents what changed: "
                "add the user-visible change, or an entry under '## Upgrading'."
            )


def main() -> int:
    version = workspace_version()
    check_sample_versions(version)
    check_surface_documented()
    check_readme_parity()

    args = sys.argv[1:]
    if args:
        if args[0] != "--since" or len(args) != 2:
            sys.exit("usage: check_docs.py [--since <git-ref>]")
        check_release_touched_readmes(args[1])

    if failures:
        print("Documentation gate failed:", file=sys.stderr)
        for item in failures:
            print(f"  - {item}", file=sys.stderr)
        return 1
    print(f"Documentation gate passed for version {version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
