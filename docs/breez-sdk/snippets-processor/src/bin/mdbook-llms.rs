//! mdbook backend writing the llmstxt.org files.
//!
//! One binary serves every `[output.llms*]` renderer; the destination directory
//! names which one is running. `book/llms` gets the index and the all-language
//! corpus, `book/llms-<edition>` gets that edition's corpus. The snippets
//! preprocessor has already reduced each edition to a single language.

use mdbook::book::Chapter;
use mdbook::errors::Result;
use mdbook::renderer::RenderContext;
use mdbook::BookItem;
use regex::Regex;
use std::fs;
use std::io;

/// Editions offered in the index: key, label, and the dependency an agent can
/// see in the project it is working on.
const EDITIONS: [(&str, &str, &str); 9] = [
    ("rust", "Rust", "the `breez-sdk-spark` crate"),
    ("swift", "Swift", "the `breez-sdk-spark-swift` Swift package"),
    (
        "kotlin",
        "Kotlin",
        "`technology.breez.spark:breez-sdk-spark-kmp` or `breez_sdk_spark:bindings-android`",
    ),
    ("csharp", "C#", "the `Breez.Sdk.Spark` NuGet package"),
    (
        "wasm",
        "Javascript (Wasm)",
        "the `@breeztech/breez-sdk-spark` npm package",
    ),
    (
        "react-native",
        "React Native",
        "the `@breeztech/breez-sdk-spark-react-native` npm package",
    ),
    ("flutter", "Flutter", "the `breez_sdk_spark_flutter` pub package"),
    ("python", "Python", "the `breez-sdk-spark` PyPI package"),
    ("go", "Go", "the `github.com/breez/breez-sdk-spark-go` module"),
];

fn main() -> Result<()> {
    let ctx = RenderContext::from_json(io::stdin())?;
    fs::create_dir_all(&ctx.destination)?;

    let title = ctx
        .config
        .book
        .title
        .clone()
        .unwrap_or_else(|| "Documentation".to_string());

    // `book/llms` -> None (all languages), `book/llms-rust` -> Some("rust").
    let edition = ctx
        .destination
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("llms-"))
        .filter(|key| EDITIONS.iter().any(|(k, _, _)| k == key))
        .map(str::to_string);

    match &edition {
        Some(key) => {
            let name = format!("llms-{key}-full.txt");
            fs::write(ctx.destination.join(name), corpus(&ctx, &title, Some(key)))?;

            // One page per chapter, mirroring the site's own layout so the
            // relative links between them keep working.
            let root = ctx.destination.join("llms").join(key);
            for item in ctx.book.iter() {
                let BookItem::Chapter(chapter) = item else {
                    continue;
                };
                let Some(path) = chapter.path.as_ref() else {
                    continue;
                };
                let target = root.join(path.with_extension("md"));
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(target, chapter.content.trim().to_string() + "\n")?;
            }
            fs::write(root.join("llms.txt"), index(&ctx, &title, Some(key)))?;
        }
        None => {
            fs::write(ctx.destination.join("llms.txt"), index(&ctx, &title, None))?;
            fs::write(
                ctx.destination.join("llms-full.txt"),
                corpus(&ctx, &title, None),
            )?;
        }
    }
    Ok(())
}

/// Site path a chapter is published at. Editions live under `/llms/<key>/`,
/// the all-language pages at the site root.
fn chapter_path(chapter: &Chapter, edition: Option<&str>) -> Option<String> {
    let path = chapter.path.as_ref()?;
    let rel = path.with_extension("md");
    Some(match edition {
        Some(key) => format!("/llms/{key}/{}", rel.to_string_lossy()),
        None => format!("/{}", rel.to_string_lossy()),
    })
}

/// llmstxt.org index: H1, a summary blockquote, then one H2 per top-level
/// chapter listing it and its descendants, then the edition list.
fn index(ctx: &RenderContext, title: &str, edition: Option<&str>) -> String {
    let label = edition.and_then(edition_label);
    let mut out = match label {
        Some(label) => format!("# {title}: {label}\n\n"),
        None => format!("# {title}\n\n"),
    };
    out.push_str(&match label {
        Some(label) => format!(
            "> End-to-end SDK for instant, non-custodial bitcoin and stablecoin payments. \
             This edition carries every example in {label}, with identifiers in that \
             language's naming convention. The same pages carrying all nine languages are \
             under `/guide/`.\n\n"
        ),
        None => "> End-to-end SDK for instant, non-custodial bitcoin and stablecoin payments, \
             with bindings for nine languages. Start from the language edition matching \
             your project's dependency, listed directly below. Every page is also published \
             as markdown at the same path with a `.md` extension.\n\n"
            .to_string(),
    });

    // First, so an agent picks its language before it starts fetching pages.
    if edition.is_none() {
        out.push_str("## Language editions\n\n");
        out.push_str(
            "Each edition carries every page listed below, with examples in one language \
             only and identifiers in that language's naming convention.\n\n",
        );
        for (key, label, dependency) in EDITIONS {
            out.push_str(&format!(
                "- [{label}](/llms/{key}/llms.txt): for {dependency}. \
                 [Whole book in one file](/llms-{key}-full.txt).\n"
            ));
        }
        out.push('\n');
    }

    for item in &ctx.book.sections {
        let BookItem::Chapter(chapter) = item else {
            continue;
        };
        out.push_str(&format!("## {}\n\n", chapter.name));
        let mut links = String::new();
        collect_links(chapter, edition, &mut links);
        out.push_str(&links);
        out.push('\n');
    }

    out
}

fn edition_label(key: &str) -> Option<&'static str> {
    EDITIONS
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, label, _)| *label)
}

/// A chapter's own link followed by its descendants', depth-first.
fn collect_links(chapter: &Chapter, edition: Option<&str>, out: &mut String) {
    if let Some(path) = chapter_path(chapter, edition) {
        out.push_str(&format!("- [{}]({})\n", chapter.name, path));
    }
    for item in &chapter.sub_items {
        if let BookItem::Chapter(child) = item {
            collect_links(child, edition, out);
        }
    }
}

/// Make a page's relative links absolute.
///
/// A corpus is served from the site root, so the relative links a page uses to
/// reach its neighbours would resolve against the wrong directory.
fn absolutise(content: &str, base: &str) -> String {
    let link = Regex::new(r"\]\(([^)]+)\)").unwrap();
    let mut out = Vec::new();
    let mut in_code = false;

    for line in content.split('\n') {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            out.push(line.to_string());
            continue;
        }
        if in_code {
            out.push(line.to_string());
            continue;
        }
        out.push(
            link.replace_all(line, |caps: &regex::Captures| {
                let target = &caps[1];
                let already_resolved = target.starts_with('/')
                    || target.starts_with('#')
                    || target.starts_with("http://")
                    || target.starts_with("https://")
                    || target.starts_with("mailto:");
                if already_resolved {
                    return format!("]({target})");
                }
                format!("]({base}/{})", target.strip_prefix("./").unwrap_or(target))
            })
            .into_owned(),
        );
    }
    out.join("\n")
}

/// Every chapter's content, each preceded by a link to its canonical page.
fn corpus(ctx: &RenderContext, title: &str, edition: Option<&str>) -> String {
    let mut out = format!("# {title}\n\n");
    for item in ctx.book.iter() {
        let BookItem::Chapter(chapter) = item else {
            continue;
        };
        if chapter.content.trim().is_empty() {
            continue;
        }
        let Some(path) = chapter_path(chapter, edition) else {
            continue;
        };
        out.push_str(&format!("**→ [{}]({})**\n\n", chapter.name, path));

        let base = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        out.push_str(absolutise(chapter.content.trim(), base).trim());
        out.push_str("\n\n");
    }
    out
}
