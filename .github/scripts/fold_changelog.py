#!/usr/bin/env python3
"""Fold release-please's package changelog into the repo-root CHANGELOG.md.

release-please resolves `changelog-path` relative to its package directory
and rejects `..`, so it can't be pointed at the repo root directly. Instead
it writes its default `crates/forgeguard-cli/CHANGELOG.md` and this script
splices that section into the root file, prefixes the heading with the product
name, and removes the package copy so the changelog only ever lives at the root.

Re-runnable: release-please recreates its release branch as new commits land
on main, so a section for the same version is replaced rather than
duplicated or left stale.
"""

import os
import re
import sys

HEADING = "# Changelog"


def split_sections(body):
    """Split markdown into (preamble, [section, ...]) on `## ` headings."""
    parts = re.split(r"^(?=## )", body, flags=re.MULTILINE)
    if not parts:
        return "", []
    if parts[0].startswith("## "):
        return "", parts
    return parts[0], parts[1:]


def strip_top_heading(text):
    """Drop a leading `# ...` title and the blank lines under it."""
    lines = text.splitlines(keepends=True)
    if lines and lines[0].startswith("# "):
        lines = lines[1:]
        while lines and not lines[0].strip():
            lines = lines[1:]
    return "".join(lines)


def parse_version(section):
    """Read the released version off the generated section's heading."""
    match = re.search(r"^## (?:.+: )?\[([^\]]+)\]", section, flags=re.MULTILINE)
    if not match:
        raise SystemExit("generated changelog has no version heading")
    return match.group(1)


def extract_release_section(src_text, version=None):
    """Extract (version, section) from src_text containing one or more sections."""
    _, sections = split_sections(strip_top_heading(src_text))
    if not sections:
        stripped = strip_top_heading(src_text).strip("\n")
        if not stripped:
            raise SystemExit("generated changelog is empty")
        v = parse_version(stripped)
        return v, stripped

    if version:
        marker = f"## [{version}]"
        for s in sections:
            if s.startswith(marker) or re.match(r"^## (?:.+: )?\[" + re.escape(version) + r"\]", s):
                return version, s.strip("\n")
        raise SystemExit(f"generated changelog has no section for version {version}")

    for s in sections:
        m = re.match(r"^## (?:.+: )?\[([^\]]+)\]", s)
        if m:
            return m.group(1), s.strip("\n")

    raise SystemExit("generated changelog has no version heading")


def fold(src_text, dst_text, version=None, product="ForgeGuard"):
    stripped = strip_top_heading(src_text).strip("\n")
    if not stripped:
        raise SystemExit("generated changelog is empty")

    version, section = extract_release_section(src_text, version=version)

    marker = f"## [{version}]"
    prefixed = f"## {product}: [{version}]" if product else marker
    if section.startswith(marker) and product:
        section = prefixed + section[len(marker):]
    elif not (section.startswith(marker) or section.startswith(prefixed)):
        raise SystemExit(
            f"generated section does not start with a {version} heading"
        )

    preamble, sections = split_sections(strip_top_heading(dst_text))
    sections = [
        s
        for s in sections
        if not (s.startswith(marker) or s.startswith(prefixed))
    ]

    body = "".join(sections).lstrip("\n")
    out = f"{HEADING}\n\n{section}\n\n"
    if preamble.strip():
        out = f"{HEADING}\n\n{preamble.strip()}\n\n{section}\n\n"
    if body:
        out += body
    return out.rstrip("\n") + "\n"


def main():
    product = os.environ.get("PRODUCT_NAME", "ForgeGuard")
    src = sys.argv[1] if len(sys.argv) > 1 else "crates/forgeguard-cli/CHANGELOG.md"
    dst = sys.argv[2] if len(sys.argv) > 2 else "CHANGELOG.md"

    if not os.path.exists(src):
        print(f"{src} not present; nothing to fold")
        return

    with open(src, encoding="utf-8") as fh:
        src_text = fh.read()
    with open(dst, encoding="utf-8") as fh:
        dst_text = fh.read()

    version, _ = extract_release_section(src_text)
    folded = fold(src_text, dst_text, version=version, product=product)
    with open(dst, "w", encoding="utf-8") as fh:
        fh.write(folded)
    print(f"folded {version} into {dst}")


if __name__ == "__main__":
    main()
