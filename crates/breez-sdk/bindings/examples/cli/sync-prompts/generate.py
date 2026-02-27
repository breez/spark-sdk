#!/usr/bin/env python3
"""
Sync-prompt generator — produces GitHub Actions workflow YAML for CLI sync workflows.

Reads a shared prompt template and per-language config (TOML) to generate
the final workflow YAML files under .github/workflows/.

Usage:
    python generate.py                    # Generate all languages
    python generate.py go                 # Generate only Go
    python generate.py python go          # Generate Python and Go
    python generate.py --check            # Verify generated files are up-to-date
    python generate.py --dry-run go       # Print Go workflow to stdout
"""

from __future__ import annotations

import argparse
import difflib
import sys
import textwrap
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
REPO_ROOT = SCRIPT_DIR.parents[5]  # Navigate up from sync-prompts/ to repo root
WORKFLOWS_DIR = REPO_ROOT / ".github" / "workflows"
PROMPT_TEMPLATE = SCRIPT_DIR / "prompt-template.md"
WORKFLOW_TEMPLATE = SCRIPT_DIR / "workflow-template.yml"
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

    # Post-process: indent allowed_tools continuation lines for YAML >- scalar
    if "allowed_tools" in flat:
        lines = [l.strip() for l in flat["allowed_tools"].strip().splitlines()]
        flat["allowed_tools"] = ("\n" + " " * 12).join(lines)

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


def generate_prompt(lang_id: str) -> str:
    """Generate the rendered prompt for a language."""
    config = load_lang_config(lang_id)
    template = PROMPT_TEMPLATE.read_text()
    return render_template(template, config)


def generate_workflow(lang_id: str) -> str:
    """Generate the complete workflow YAML for a language."""
    config = load_lang_config(lang_id)

    # Render the prompt first
    prompt_text = generate_prompt(lang_id)
    # Indent prompt by 12 spaces for YAML embedding (inside `prompt: |`)
    indented_prompt = textwrap.indent(prompt_text.strip(), "            ")

    # Add rendered prompt to config for workflow template substitution
    config["prompt"] = indented_prompt

    # Render the workflow template
    workflow_template = WORKFLOW_TEMPLATE.read_text()
    return render_template(workflow_template, config)


def available_languages() -> list[str]:
    """List available language configs."""
    return sorted(p.stem for p in LANGS_DIR.glob("*.toml"))


def workflow_path(lang_id: str) -> Path:
    """Return the expected workflow file path for a language."""
    config = load_lang_config(lang_id)
    concurrency_group = config.get("concurrency_group", f"sync-{lang_id}-cli")
    # Derive filename from concurrency group
    return WORKFLOWS_DIR / f"{concurrency_group}.yml"


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate CLI sync workflow YAML from templates"
    )
    parser.add_argument(
        "languages",
        nargs="*",
        help="Languages to generate (default: all)",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Verify generated files are up-to-date (exit 1 if not)",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print generated workflows to stdout instead of writing files",
    )
    args = parser.parse_args()

    languages = args.languages or available_languages()

    # Validate languages
    available = set(available_languages())
    for lang in languages:
        if lang not in available:
            sys.exit(
                f"Error: unknown language '{lang}'. Available: {', '.join(sorted(available))}"
            )

    all_ok = True

    for lang in languages:
        generated = generate_workflow(lang)
        output_path = workflow_path(lang)

        if args.dry_run:
            print(f"# === {output_path.name} ===")
            print(generated)
            print()
            continue

        if args.check:
            if not output_path.exists():
                print(f"MISSING: {output_path}")
                all_ok = False
                continue

            existing = output_path.read_text()
            if existing != generated:
                diff = difflib.unified_diff(
                    existing.splitlines(keepends=True),
                    generated.splitlines(keepends=True),
                    fromfile=f"{output_path.name} (current)",
                    tofile=f"{output_path.name} (generated)",
                )
                print(f"OUT OF DATE: {output_path}")
                sys.stdout.writelines(diff)
                all_ok = False
            else:
                print(f"OK: {output_path}")
            continue

        # Write mode
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(generated)
        print(f"Generated: {output_path}")

    if args.check and not all_ok:
        print(
            "\nWorkflow files are out of date. Run `python generate.py` to regenerate."
        )
        sys.exit(1)


if __name__ == "__main__":
    main()
