# Introduction

The SDK docs are live at [https://sdk-doc-spark.breez.technology](https://sdk-doc-spark.breez.technology).

## Contributions

For syntax and supported features, see [https://rust-lang.github.io/mdBook](https://rust-lang.github.io/mdBook).

## Develop

To locally serve the docs run:

```bash
cargo install mdbook@0.4.52
cargo install --path ./snippets-processor
cargo install mdbook-variables
cargo install mdbook-pagetoc
mdbook build
mdbook serve --open
```

`snippets-processor` installs two binaries: `mdbook-snippets`, the preprocessor that
expands `{{#tabs}}`, `{{#name}}` and `{{#enum}}`, and `mdbook-llms`, the backend that
writes the [llmstxt.org](https://llmstxt.org) files.

## Build output

`mdbook build` writes one directory per renderer under `book/`, which the publish
workflow flattens to the site root:

| Directory | Published as | Contents |
|---|---|---|
| `html` | `/` | the website |
| `markdown` | `/guide/*.md` | every page as markdown, examples in all nine languages |
| `llms-index` | `/llms.txt`, `/llms-full.txt` | the index and the all-language corpus |
| `llms-<edition>` | `/llms-<edition>-full.txt`, `/llms/<edition>/` | one corpus and one page set per language |

Editions carry examples in a single language, with identifiers in that language's naming
convention. The list of them lives in `EDITIONS` in `snippets-processor/src/bin/mdbook-llms.rs`.
