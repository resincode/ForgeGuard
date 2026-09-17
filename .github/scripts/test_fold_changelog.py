#!/usr/bin/env python3
"""Self-check for fold_changelog.py. Run: python3 test_fold_changelog.py"""

import sys
from fold_changelog import extract_release_section, fold, parse_version

SRC_CLEAN = """# Changelog

## [0.16.0](https://github.com/suiflex/ForgeGuard/compare/v0.15.0...v0.16.0) (2026-09-17)

### Features

* **cli:** serve and register ForgeGuard over MCP ([dc564b4](https://github.com/suiflex/ForgeGuard/commit/dc564b4))
"""

SRC_WITH_UNRELEASED = """# Changelog

## Unreleased

### Features

* add benchmark harness

## [0.16.0](https://github.com/suiflex/ForgeGuard/compare/v0.15.0...v0.16.0) (2026-09-17)

### Features

* **cli:** serve and register ForgeGuard over MCP ([dc564b4](https://github.com/suiflex/ForgeGuard/commit/dc564b4))
"""

DST_EXISTING = """# Changelog

All notable changes to ForgeGuard will be documented here by Release Please.

## [0.15.0](https://github.com/suiflex/ForgeGuard/compare/v0.14.0...v0.15.0) (2026-08-29)

### Features

* **cli:** execute in-place installation on forgeguard update ([e441acd](https://github.com/suiflex/ForgeGuard/commit/e441acd))
"""


def test_parse_version_single_section():
    assert parse_version("## [0.16.0] (2026-09-17)") == "0.16.0"
    assert parse_version("## ForgeGuard: [0.16.0] (2026-09-17)") == "0.16.0"


def test_extract_clean_single_section():
    version, section = extract_release_section(SRC_CLEAN)
    assert version == "0.16.0"
    assert section.startswith("## [0.16.0]")
    assert "serve and register" in section


def test_extract_with_unreleased_header():
    version, section = extract_release_section(SRC_WITH_UNRELEASED)
    assert version == "0.16.0"
    assert section.startswith("## [0.16.0]")
    assert "add benchmark harness" not in section


def test_fold_prefixes_product_heading():
    out = fold(SRC_CLEAN, DST_EXISTING, product="ForgeGuard")
    assert "## ForgeGuard: [0.16.0]" in out
    assert "## [0.15.0]" in out
    assert "All notable changes to ForgeGuard" in out


def test_fold_with_unreleased_in_src():
    out = fold(SRC_WITH_UNRELEASED, DST_EXISTING, product="ForgeGuard")
    assert "## ForgeGuard: [0.16.0]" in out
    assert "## [0.15.0]" in out


def test_fold_replaces_existing_version():
    once = fold(SRC_CLEAN, DST_EXISTING, product="ForgeGuard")
    twice = fold(SRC_CLEAN, once, product="ForgeGuard")
    assert once == twice
    assert twice.count("## ForgeGuard: [0.16.0]") == 1


def test_missing_version_raises():
    invalid = "# Changelog\n\n## Unreleased\n\n* nothing released yet"
    try:
        extract_release_section(invalid)
        assert False, "expected SystemExit"
    except SystemExit as exc:
        assert "no version heading" in str(exc)


if __name__ == "__main__":
    tests = [
        test_parse_version_single_section,
        test_extract_clean_single_section,
        test_extract_with_unreleased_header,
        test_fold_prefixes_product_heading,
        test_fold_with_unreleased_in_src,
        test_fold_replaces_existing_version,
        test_missing_version_raises,
    ]
    passed = 0
    for run in tests:
        run()
        print(f"ok   {run.__name__}")
        passed += 1
    print(f"---\n{passed} passed, 0 failed")
