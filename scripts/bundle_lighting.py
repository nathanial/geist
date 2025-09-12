#!/usr/bin/env python3
import os
import re
import sys
from datetime import datetime
from pathlib import Path

# Configuration
REPO_ROOT = Path(__file__).resolve().parents[1]
OUTPUT_PATH = REPO_ROOT / "LightingBundle.md"

# Directories to skip entirely
SKIP_DIRS = {
    ".git",
    "target",
    "old-codebase",
    "assets",  # mostly textures/palette, not code
    "showcase_output",
    "schematics",
    "worlds",
    ".claude",
}

# File extensions we consider as code/context
CODE_EXTS = {
    ".rs": "rust",
    ".toml": "toml",
    ".ron": "ron",
}

# Heuristic patterns indicating lighting-related code
PATTERNS = [
    r"\bgeist_lighting\b",
    r"\bLighting(Store|Border|s|)\b",
    r"\bLight(Borders|Emitter|ing|)\b",
    r"\bRebuildCause::LightingBorder\b",
]
COMPILED = [re.compile(p) for p in PATTERNS]


def is_skipped_dir(path: Path) -> bool:
    parts = set(path.parts)
    return any(p in SKIP_DIRS for p in parts)


def file_matches(path: Path) -> bool:
    if not path.is_file():
        return False
    if path.suffix not in CODE_EXTS:
        return False
    try:
        text = path.read_text(encoding="utf-8", errors="ignore")
    except Exception:
        return False
    return any(rx.search(text) for rx in COMPILED)


def discover_files() -> list[Path]:
    files: list[Path] = []

    # 1) Include the geist-lighting crate entirely (primary target)
    lighting_crate = REPO_ROOT / "crates" / "geist-lighting"
    if lighting_crate.exists():
        for p in lighting_crate.rglob("*"):
            if p.is_file() and p.suffix in CODE_EXTS:
                files.append(p)

    # 2) Scan remaining repo for integration points
    for root, dirs, filenames in os.walk(REPO_ROOT):
        root_path = Path(root)
        # Skip unwanted directories
        if is_skipped_dir(root_path):
            # Prune dirs in-place
            dirs[:] = []
            continue
        # Always skip the lighting crate (already included)
        if (lighting_crate in [root_path] or str(root_path).startswith(str(lighting_crate))):
            continue

        for fname in filenames:
            p = root_path / fname
            if p.suffix in CODE_EXTS and file_matches(p):
                files.append(p)

    # Normalize and de-duplicate while preserving order
    seen = set()
    ordered: list[Path] = []
    for p in files:
        rp = p.resolve()
        if rp not in seen:
            seen.add(rp)
            ordered.append(p)
    return ordered


def language_for(path: Path) -> str:
    return CODE_EXTS.get(path.suffix, "")


def write_bundle(files: list[Path]) -> None:
    rel_paths = [p.relative_to(REPO_ROOT) for p in files]
    ts = datetime.now().isoformat(timespec="seconds")

    lines: list[str] = []
    lines.append("# Lighting Code Bundle")
    lines.append("")
    lines.append(f"Generated: {ts}")
    lines.append(f"Repository: {REPO_ROOT}")
    lines.append("")
    lines.append("This file aggregates lighting-related code for review and optimization.")
    lines.append("")

    # Table of contents
    lines.append("## Table of Contents")
    for i, rp in enumerate(rel_paths, start=1):
        lines.append(f"- [{rp}] (#file-{i})")
    lines.append("")

    # Files with code blocks
    for i, (rp, p) in enumerate(zip(rel_paths, files), start=1):
        lines.append(f"---")
        lines.append("")
        lines.append(f"## {rp}")
        lines.append(f"<a id=\"file-{i}\"></a>")
        lines.append("")
        lang = language_for(p)
        try:
            content = p.read_text(encoding="utf-8", errors="ignore")
        except Exception as e:
            content = f"<Error reading file: {e}>"
            lang = ""
        lines.append(f"```{lang}")
        lines.append(content)
        lines.append("```")
        lines.append("")

    OUTPUT_PATH.write_text("\n".join(lines), encoding="utf-8")


def main(argv: list[str]) -> int:
    files = discover_files()
    if not files:
        print("No lighting-related files found.")
        return 1
    write_bundle(files)
    print(f"Wrote {len(files)} files to {OUTPUT_PATH}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))

