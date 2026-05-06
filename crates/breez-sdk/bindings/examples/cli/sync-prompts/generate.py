#!/usr/bin/env python3
"""
Sync-prompt generator for CLI sync workflows.

Reads a shared prompt template and per-language config (TOML) to produce
rendered prompts.  The sync workflow (.github/workflows/sync-cli.yml) calls
generate_prompt() at runtime to assemble the prompt for each language.

Usage:
    python generate.py --prompt-only dart   # Print rendered prompt for Dart
    python generate.py --prompt-only go     # Print rendered prompt for Go
    python generate.py --list               # List available languages
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

# ---------------------------------------------------------------------------
# Minimal TOML parser (no external dependencies)
# ---------------------------------------------------------------------------
# Handles flat keys, [sections], and multi-line strings (""" ... """).
# Sufficient for our simple config format.

def parse_toml(text: str) -> dict[str, dict[str, str]]:
    """Parse a minimal TOML file into {section: {key: value}} dict."""
    result: dict[str, dict[str, str]] = {}
    current_section = "meta"
    result[current_section] = {}

    lines = text.split("\n")
    i = 0
    while i < len(lines):
        line = lines[i].strip()

        # Skip comments and blank lines
        if not line or line.startswith("#"):
            i += 1
            continue

        # Section header
        if line.startswith("[") and line.endswith("]"):
            current_section = line[1:-1]
            if current_section not in result:
                result[current_section] = {}
            i += 1
            continue

        # Key = value
        if "=" in line:
            key, _, value = line.partition("=")
            key = key.strip()
            value = value.strip()

            # Multi-line string (triple-quoted)
            if value.startswith('"""'):
                value_content = value[3:]
                if value_content.endswith('"""'):
                    # Single-line triple-quoted
                    result[current_section][key] = value_content[:-3]
                else:
                    # Multi-line: collect until closing """
                    parts = [value_content]
                    i += 1
                    while i < len(lines):
                        if lines[i].rstrip().endswith('"""'):
                            parts.append(lines[i].rstrip()[:-3])
                            break
                        parts.append(lines[i].rstrip())
                        i += 1
                    result[current_section][key] = "\n".join(parts)
            # Regular quoted string
            elif value.startswith('"') and value.endswith('"'):
                result[current_section][key] = value[1:-1]
            # Single-quoted string
            elif value.startswith("'") and value.endswith("'"):
                result[current_section][key] = value[1:-1]
            # Unquoted value
            else:
                result[current_section][key] = value

            i += 1
            continue

        i += 1

    return result


# ---------------------------------------------------------------------------
# Generator
# ---------------------------------------------------------------------------

SCRIPT_DIR = Path(__file__).resolve().parent
PROMPT_TEMPLATE = SCRIPT_DIR / "prompt-template.md"
LANGS_DIR = SCRIPT_DIR / "langs"


def load_lang_config(lang_id: str) -> dict[str, str]:
    """Load and flatten a language TOML config into a simple key→value dict."""
    config_path = LANGS_DIR / f"{lang_id}.toml"
    if not config_path.exists():
        sys.exit(f"Error: language config not found: {config_path}")

    parsed = parse_toml(config_path.read_text())

    # Flatten all sections into a single dict.
    # Keys from [meta] go in as-is; keys from other sections are prefixed
    # with the section name (e.g., [file_mapping].content → file_mapping).
    # If a section has a single "content" key, use just the section name.
    flat: dict[str, str] = {}
    for section_name, section_dict in parsed.items():
        if section_name == "meta":
            for key, value in section_dict.items():
                flat[key] = value
        else:
            for key, value in section_dict.items():
                if key == "content":
                    flat[section_name] = value
                else:
                    flat[f"{section_name}_{key}"] = value

    return flat


def render_template(template: str, variables: dict[str, str]) -> str:
    """Replace {{VARIABLE}} placeholders with values from the dict.

    Only replaces variables wrapped in double curly braces ({{VAR}}).
    GitHub Actions expressions (${{ ... }}) are left untouched because
    they use a different prefix ($).
    """
    result = template
    for key, value in variables.items():
        placeholder = "{{" + key.upper() + "}}"
        result = result.replace(placeholder, value)
    return result


def generate_prompt(lang_id: str, extra_vars: dict[str, str] | None = None) -> str:
    """Generate the rendered prompt for a language.

    `extra_vars` are merged into the per-language config before rendering,
    used by the sync workflow to inject runtime values like the diff
    summary and base SHA. Keys are uppercased to match {{KEY}} placeholders.
    """
    config = load_lang_config(lang_id)
    if extra_vars:
        config = {**config, **extra_vars}
    template = PROMPT_TEMPLATE.read_text()
    return render_template(template, config)


def available_languages() -> list[str]:
    """List available language configs."""
    return sorted(p.stem for p in LANGS_DIR.glob("*.toml"))


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate CLI sync prompts from templates"
    )
    parser.add_argument(
        "languages",
        nargs="*",
        help="Languages to process (default: all)",
    )
    parser.add_argument(
        "--prompt-only",
        action="store_true",
        help="Print rendered prompt(s) to stdout",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="List available languages and exit",
    )
    args = parser.parse_args()

    if args.list:
        for lang in available_languages():
            print(lang)
        return

    languages = args.languages or available_languages()

    # Validate languages
    available = set(available_languages())
    for lang in languages:
        if lang not in available:
            sys.exit(
                f"Error: unknown language '{lang}'. Available: {', '.join(sorted(available))}"
            )

    if args.prompt_only:
        for lang in languages:
            prompt = generate_prompt(lang)
            if len(languages) > 1:
                print(f"# === {lang} ===")
            print(prompt)
            if len(languages) > 1:
                print()
    else:
        parser.print_help()
        print(f"\nAvailable languages: {', '.join(available_languages())}")
        print("\nUse --prompt-only to generate prompts.")


if __name__ == "__main__":
    main()
