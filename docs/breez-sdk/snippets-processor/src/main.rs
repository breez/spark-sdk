use clap::{crate_version, Arg, ArgMatches, Command};
use mdbook::book::Book;
use mdbook::errors::{Error, Result};
use mdbook::preprocess::{CmdPreprocessor, Preprocessor, PreprocessorContext};
use mdbook::BookItem;
use std::fs;
use std::io;

fn main() -> Result<()> {
    // set up app
    let matches = make_app().get_matches();
    let pre = SnippetsProcessor;

    // determine what behaviour has been requested
    if let Some(sub_args) = matches.subcommand_matches("supports") {
        // handle cmdline supports
        handle_supports(&pre, sub_args)
    } else {
        // handle preprocessing
        handle_preprocessing(&pre)
    }
}

/// Parse CLI options.
pub fn make_app() -> Command {
    Command::new("mdbook-snippets")
        .version(crate_version!())
        .about("A preprocessor that removes leading whitespace from code snippets.")
        .subcommand(
            Command::new("supports")
                .arg(Arg::new("renderer").required(true))
                .about("Check whether a renderer is supported by this preprocessor"),
        )
}

/// Tell mdBook if we support what it asks for.
fn handle_supports(pre: &dyn Preprocessor, sub_args: &ArgMatches) -> Result<()> {
    let renderer = sub_args
        .get_one::<String>("renderer")
        .expect("Required argument");
    let supported = pre.supports_renderer(renderer);
    if supported {
        Ok(())
    } else {
        Err(Error::msg(format!(
            "The snippets preprocessor does not support the '{renderer}' renderer",
        )))
    }
}

/// Preprocess `book` using `pre` and print it out.
fn handle_preprocessing(pre: &dyn Preprocessor) -> Result<()> {
    let (ctx, book) = CmdPreprocessor::parse_input(io::stdin())?;
    check_mdbook_version(&ctx.mdbook_version);

    let processed_book = pre.run(&ctx, book)?;
    serde_json::to_writer(io::stdout(), &processed_book)?;
    Ok(())
}

/// Produce a warning on mdBook version mismatch.
fn check_mdbook_version(version: &str) {
    if version != mdbook::MDBOOK_VERSION {
        eprintln!(
            "This mdbook-snippets was built against mdbook v{}, \
            but we are being called from mdbook v{version}. \
            If you have any issue, this might be a reason.",
            mdbook::MDBOOK_VERSION,
        )
    }
}

/// Appended to non-HTML output, where `{{#name}}` and `{{#enum}}` render as one
/// Rust-cased identifier rather than one span per language.
const CASING_FOOTER: &str = "\n\n---\n\nIdentifier casing: `get_info` here is `getInfo` \
in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. \
Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in \
Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.\n";

/// A single-language edition: which snippet tab to keep, and how that
/// language writes identifiers.
#[derive(Clone, Copy)]
struct Edition {
    key: &'static str,
    /// Matching tab label in `get_language_paths`.
    tab: &'static str,
}

const EDITIONS: [Edition; 9] = [
    Edition {
        key: "rust",
        tab: "Rust",
    },
    Edition {
        key: "swift",
        tab: "Swift",
    },
    Edition {
        key: "kotlin",
        tab: "Kotlin",
    },
    Edition {
        key: "csharp",
        tab: "C#",
    },
    Edition {
        key: "wasm",
        tab: "Javascript",
    },
    Edition {
        key: "react-native",
        tab: "React Native",
    },
    Edition {
        key: "flutter",
        tab: "Flutter",
    },
    Edition {
        key: "python",
        tab: "Python",
    },
    Edition {
        key: "go",
        tab: "Go",
    },
];

impl Edition {
    fn from_renderer(renderer: &str) -> Option<Edition> {
        let key = renderer.strip_prefix("llms-")?;
        EDITIONS.iter().copied().find(|e| e.key == key)
    }

    fn case(&self, s: &str) -> String {
        match self.key {
            "rust" | "python" => s.to_string(),
            "go" | "csharp" => capitalize_first(s),
            _ => SnippetsProcessor::to_camel_case(s),
        }
    }

    fn name(&self, identifier: &str) -> String {
        match identifier.split_once('.') {
            // An uppercase-initial prefix is a type name, identical everywhere.
            Some((prefix, method)) if prefix.starts_with(|c: char| c.is_uppercase()) => {
                format!("{prefix}.{}", self.case(method))
            }
            Some((prefix, method)) => format!("{}.{}", self.case(prefix), self.case(method)),
            None => self.case(identifier),
        }
    }

    fn enum_variant(&self, type_name: &str, variant: &str) -> String {
        match self.key {
            "rust" => format!("{type_name}::{variant}"),
            "python" => format!(
                "{type_name}.{}",
                SnippetsProcessor::to_screaming_snake(variant)
            ),
            "swift" => format!("{type_name}.{}", SnippetsProcessor::to_lower_camel(variant)),
            "go" => format!("{type_name}{variant}"),
            _ => format!("{type_name}.{variant}"),
        }
    }
}

struct SnippetsProcessor;
impl SnippetsProcessor {
    /// Convert snake_case to camelCase
    fn to_camel_case(s: &str) -> String {
        let mut result = String::new();
        let mut capitalize_next = false;
        for ch in s.chars() {
            if ch == '_' {
                capitalize_next = true;
            } else if capitalize_next {
                result.extend(ch.to_uppercase());
                capitalize_next = false;
            } else {
                result.push(ch);
            }
        }
        result
    }

    /// Convert PascalCase to SCREAMING_SNAKE_CASE (for Python enums)
    fn to_screaming_snake(s: &str) -> String {
        let mut result = String::new();
        for (i, ch) in s.chars().enumerate() {
            if ch.is_uppercase() && i > 0 {
                result.push('_');
            }
            result.extend(ch.to_uppercase());
        }
        result
    }

    /// Convert PascalCase to camelCase (for Swift enum cases)
    fn to_lower_camel(s: &str) -> String {
        let mut chars = s.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        }
    }

    /// Expand {{#name identifier}} to language-aware HTML.
    ///
    /// Supports two dotted shapes:
    /// - `Type.method` — prefix starts uppercase, treated as a type/class
    ///   name and emitted verbatim across all languages.
    /// - `field.subfield` — prefix starts lowercase, treated as a snake_case
    ///   field path and case-converted alongside the trailing segment.
    fn expand_name(identifier: &str, markdown: bool, edition: Option<Edition>) -> String {
        if let Some(edition) = edition {
            return format!("`{}`", edition.name(identifier));
        }
        if markdown {
            return format!("`{identifier}`");
        }

        let (type_prefix, method) = identifier
            .split_once('.')
            .map(|(t, m)| (Some(t), m))
            .unwrap_or((None, identifier));

        let snake = method.to_string();
        let camel = Self::to_camel_case(method);
        let pascal = capitalize_first(method);

        // Pre-compute prefix in each casing flavor. For a Type prefix
        // (starts uppercase), all three are identical to the original. For a
        // field prefix (starts lowercase), they follow the method's casing
        // per language.
        let prefix_cases = type_prefix.map(|t| {
            let is_type = t.starts_with(|c: char| c.is_uppercase());
            if is_type {
                (t.to_string(), t.to_string(), t.to_string())
            } else {
                (
                    t.to_string(),                     // snake (rust, python)
                    Self::to_camel_case(t),            // camel (swift, kotlin, js, rn, flutter)
                    capitalize_first(t),               // pascal (go, csharp)
                )
            }
        });

        // (language tag, method-casing, prefix-casing)
        let variants: [(&str, &String, Option<&String>); 9] = [
            ("rust", &snake, prefix_cases.as_ref().map(|p| &p.0)),
            ("python", &snake, prefix_cases.as_ref().map(|p| &p.0)),
            ("swift", &camel, prefix_cases.as_ref().map(|p| &p.1)),
            ("kotlin", &camel, prefix_cases.as_ref().map(|p| &p.1)),
            ("javascript", &camel, prefix_cases.as_ref().map(|p| &p.1)),
            ("react-native", &camel, prefix_cases.as_ref().map(|p| &p.1)),
            ("flutter", &camel, prefix_cases.as_ref().map(|p| &p.1)),
            ("go", &pascal, prefix_cases.as_ref().map(|p| &p.2)),
            ("csharp", &pascal, prefix_cases.as_ref().map(|p| &p.2)),
        ];

        let spans: String = variants
            .iter()
            .map(|(lang, method_name, prefix)| {
                let display = match prefix {
                    Some(p) => format!("{}.{}", p, method_name),
                    None => (*method_name).clone(),
                };
                format!("<span class=\"fn-{}\">{}</span>", lang, display)
            })
            .collect();

        format!("<code class=\"lang-fn\">{}</code>", spans)
    }

    /// Expand {{#enum Type::Variant}} to language-aware HTML
    fn expand_enum(enum_str: &str, markdown: bool, edition: Option<Edition>) -> String {
        if let Some(edition) = edition {
            return match enum_str.split_once("::") {
                Some((type_name, variant)) => {
                    format!("`{}`", edition.enum_variant(type_name, variant))
                }
                None => format!("`{enum_str}`"),
            };
        }
        if markdown {
            return format!("`{enum_str}`");
        }

        // Parse Type::Variant
        let (type_name, variant) = enum_str
            .split_once("::")
            .unwrap_or((enum_str, ""));

        if variant.is_empty() {
            // No variant found, just return as-is
            return format!("<code class=\"lang-fn\">{}</code>", enum_str);
        }

        let variants = [
            ("rust", format!("{}::{}", type_name, variant)),
            ("python", format!("{}.{}", type_name, Self::to_screaming_snake(variant))),
            ("swift", format!("{}.{}", type_name, Self::to_lower_camel(variant))),
            ("kotlin", format!("{}.{}", type_name, variant)),
            ("javascript", format!("{}.{}", type_name, variant)),
            ("react-native", format!("{}.{}", type_name, variant)),
            ("flutter", format!("{}.{}", type_name, variant)),
            ("go", format!("{}{}", type_name, variant)),
            ("csharp", format!("{}.{}", type_name, variant)),
        ];

        let spans: String = variants
            .iter()
            .map(|(lang, display)| format!("<span class=\"fn-{}\">{}</span>", lang, display))
            .collect();

        format!("<code class=\"lang-fn\">{}</code>", spans)
    }

    /// Rewrite the raw `<hN id=...>` blocks carrying the API-docs link into
    /// markdown headings, so non-HTML output keeps its section structure.
    fn html_headings_to_markdown(content: &str) -> String {
        let re = regex::Regex::new(
            r#"(?s)<h([1-6])\s+id="[^"]*"\s*>\s*<a\s+class="header"[^>]*>(.*?)</a>\s*(?:<a\s+class="tag"[^>]*?href="([^"]*)"[^>]*>.*?</a>\s*)?</h[1-6]\s*>"#,
        )
        .unwrap();

        let expanded = re.replace_all(content, |caps: &regex::Captures| {
            let level = caps[1].parse::<usize>().unwrap_or(2);
            let mut out = format!("{} {}", "#".repeat(level), caps[2].trim());
            if let Some(url) = caps.get(3) {
                out.push_str(&format!("\n\nAPI docs: {}", url.as_str()));
            }
            out
        });

        // Plain `<hN id="...">Title</hN>`, used where there is no API-docs link.
        let plain =
            regex::Regex::new(r#"(?s)<h([1-6])\s+id="[^"]*"\s*>\s*(.*?)\s*</h[1-6]\s*>"#).unwrap();

        plain
            .replace_all(&expanded, |caps: &regex::Captures| {
                let level = caps[1].parse::<usize>().unwrap_or(2);
                format!("{} {}", "#".repeat(level), caps[2].trim())
            })
            .into_owned()
    }

    /// Point links at the edition's own pages, so following one from a Rust
    /// page does not land on the multi-language version.
    ///
    /// Relative links already resolve inside the edition directory. Only
    /// site-absolute `.md` links and image paths need moving; `.html` links
    /// belong to the rendered site and are left alone.
    fn rewrite_links(content: &str, edition: Edition) -> String {
        let pages = regex::Regex::new(r"\]\(/guide/([^)]*\.md[^)]*)\)").unwrap();
        let moved = pages.replace_all(
            content,
            format!("](/llms/{}/guide/$1)", edition.key).as_str(),
        );

        let images = regex::Regex::new(r"\]\(images/([^)]*)\)").unwrap();
        images
            .replace_all(&moved, "](/guide/images/$1)")
            .into_owned()
    }

    /// Unwrap `<div class="warning">` blocks, keeping the markdown inside and
    /// turning the `<h4>` title into bold text.
    fn unwrap_warning_divs(content: &str) -> String {
        let block = regex::Regex::new(r#"(?s)<div class="warning">\s*(.*?)\s*</div>"#).unwrap();
        let title = regex::Regex::new(r"(?s)<h4>\s*(.*?)\s*</h4>\s*").unwrap();

        block
            .replace_all(content, |caps: &regex::Captures| {
                title.replace_all(&caps[1], "**$1**\n\n").into_owned()
            })
            .into_owned()
    }

    fn get_language_paths(file_base: &str) -> Vec<(&'static str, &'static str, Vec<String>)> {
        vec![
            ("Rust", "rust", vec![format!("snippets/rust/src/{}.rs", file_base)]),
            ("Swift", "swift", vec![format!("snippets/swift/BreezSdkSnippets/Sources/{}.swift", capitalize_first(file_base))]),
            // Kotlin MPP: most snippets live in commonMain, but Android-only
            // ones (e.g. Passkey, which uses Credential Manager APIs) live in
            // androidMain. Try commonMain first, then fall back to androidMain.
            ("Kotlin", "kotlin", vec![
                format!("snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/{}.kt", capitalize_first(file_base)),
                format!("snippets/kotlin_mpp_lib/shared/src/androidMain/kotlin/com/example/kotlinmpplib/{}.kt", capitalize_first(file_base)),
            ]),
            ("C#", "csharp", vec![format!("snippets/csharp/{}.cs", capitalize_first(file_base))]),
            ("Javascript", "typescript", vec![format!("snippets/wasm/{}.ts", file_base)]),
            ("React Native", "typescript", vec![format!("snippets/react-native/{}.ts", file_base)]),
            ("Flutter", "dart", vec![format!("snippets/flutter/lib/{}.dart", file_base)]),
            ("Python", "python", vec![format!("snippets/python/src/{}.py", file_base)]),
            ("Go", "go", vec![format!("snippets/go/{}.go", file_base)]),
        ]
    }

    fn extract_snippet(content: &str, snippet_name: &str) -> Option<String> {
        // Try different comment styles
        let comment_styles = [
            ("// ANCHOR: ", "// ANCHOR_END: "), // Rust, Swift, Kotlin, etc.
            ("# ANCHOR: ", "# ANCHOR_END: "),   // Python
        ];

        for (start_pattern, end_pattern) in &comment_styles {
            let start_anchor = format!("{}{}", start_pattern, snippet_name);
            let end_anchor = format!("{}{}", end_pattern, snippet_name);

            if let (Some(start_pos), Some(end_pos)) =
                (content.find(&start_anchor), content.find(&end_anchor))
            {
                if start_pos >= end_pos {
                    continue;
                }

                // Find the start of the next line after the start anchor
                let start_content = start_pos + start_anchor.len();
                let snippet_start = content[start_content..]
                    .find('\n')
                    .map(|pos| start_content + pos + 1)
                    .unwrap_or(start_content);

                let raw_snippet = &content[snippet_start..end_pos];

                // Normalize indentation like mdBook does
                let lines: Vec<&str> = raw_snippet.lines().collect();
                if lines.is_empty() {
                    return Some(String::new());
                }

                // Find the minimum indentation (excluding empty lines)
                let min_indent = lines
                    .iter()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| line.len() - line.trim_start().len())
                    .min()
                    .unwrap_or(0);

                // Rebuild the snippet with normalized indentation
                let normalized_lines: Vec<String> = lines
                    .iter()
                    .map(|line| {
                        if line.trim().is_empty() {
                            String::new()
                        } else if line.len() >= min_indent {
                            line[min_indent..].to_string()
                        } else {
                            line.trim_start().to_string()
                        }
                    })
                    .collect();

                return Some(normalized_lines.join("\n").trim().to_string());
            }
        }

        None
    }

    /// The website tab reads "Javascript". In markdown, where React Native is
    /// also TypeScript, the platform has to be explicit.
    fn markdown_label(lang_name: &str) -> &str {
        match lang_name {
            "Javascript" => "Javascript (Wasm)",
            other => other,
        }
    }

    fn expand_tabs(
        ctx: &PreprocessorContext,
        file_base: &str,
        snippet_name: &str,
        markdown: bool,
        section_level: usize,
        edition: Option<Edition>,
    ) -> Result<String> {
        let config = Self::get_language_paths(file_base);
        let mut result = if markdown {
            String::new()
        } else {
            String::from("<custom-tabs category=\"lang\">\n")
        };

        for (lang_name, lang_code, relative_paths) in &config {
            // An edition keeps only its own language.
            if edition.is_some_and(|e| e.tab != *lang_name) {
                continue;
            }

            // Try each candidate path in order and use the first one that both
            // exists and contains the requested snippet anchor.
            let snippet = relative_paths.iter().find_map(|relative_path| {
                let full_path = ctx.root.join(relative_path);
                let content = fs::read_to_string(&full_path).ok()?;
                Self::extract_snippet(&content, snippet_name)
            });

            let Some(snippet) = snippet else {
                // Skip this language if no candidate file has the snippet
                continue;
            };

            if edition.is_some() {
                // One language, so a heading naming it would be noise.
                result.push_str(&format!("```{lang_code}\n{snippet}\n```\n\n"));
            } else if markdown {
                let hashes = "#".repeat((section_level + 1).min(6));
                let label = Self::markdown_label(lang_name);
                result.push_str(&format!(
                    "{hashes} {label}\n\n```{lang_code}\n{snippet}\n```\n\n"
                ));
            } else {
                result.push_str(&format!(
                    "<div slot=\"title\">{}</div>\n<section>\n\n```{},ignore\n{}\n```\n\n</section>\n\n",
                    lang_name, lang_code, snippet
                ));
            }
        }

        if !markdown {
            result.push_str("</custom-tabs>\n");
        }
        Ok(result)
    }
}

fn capitalize_first(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;

    for ch in s.chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }

    result
}

impl Preprocessor for SnippetsProcessor {
    fn name(&self) -> &str {
        "snippets"
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book> {
        // Depth of a markdown ATX heading, or None if the line is not one.
        fn heading_level(line: &str) -> Option<usize> {
            let hashes = line.len() - line.trim_start_matches('#').len();
            if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
                Some(hashes)
            } else {
                None
            }
        }

        // The HTML renderer gets language tabs and one identifier span per
        // language. Every other renderer gets plain markdown instead.
        let markdown = ctx.renderer != "html";
        let edition = Edition::from_renderer(&ctx.renderer);

        book.for_each_mut(|item| {
            if let BookItem::Chapter(chapter) = item {
                if markdown {
                    chapter.content = Self::html_headings_to_markdown(&chapter.content);
                    chapter.content = Self::unwrap_warning_divs(&chapter.content);
                }
                if let Some(edition) = edition {
                    chapter.content = Self::rewrite_links(&chapter.content, edition);
                }

                let mut used_identifier_macro = false;
                let mut section_level = 1usize;
                let mut resulting_lines: Vec<String> = vec![];
                let mut in_block = false;
                let mut block_lines: Vec<String> = vec![];
                let mut min_indentation: usize = 0;

                // Compile regexes once outside the loop
                let name_regex = regex::Regex::new(r"\{\{#name\s+([\w.]+)\}\}").unwrap();
                let enum_regex = regex::Regex::new(r"\{\{#enum\s+([\w:]+)\}\}").unwrap();

                for line in chapter.content.lines() {
                    // Check for tab expansion syntax: {{#tabs file:snippet-name}}
                    if let Some(captures) = regex::Regex::new(r"\{\{#tabs\s+([^:]+):([\w-]+)\}\}")
                        .unwrap()
                        .captures(line)
                    {
                        if let (Some(file_base), Some(snippet_name)) =
                            (captures.get(1), captures.get(2))
                        {
                            match Self::expand_tabs(
                                ctx,
                                file_base.as_str(),
                                snippet_name.as_str(),
                                markdown,
                                section_level,
                                edition,
                            ) {
                                Ok(expanded) => {
                                    resulting_lines.push(expanded);
                                }
                                Err(e) => {
                                    eprintln!("Error expanding tabs: {}", e);
                                    // Keep the original line on error
                                    resulting_lines.push(line.to_string());
                                }
                            }
                            continue;
                        }
                    }

                    // Check for {{#name}} and {{#enum}} patterns
                    // Handle multiple occurrences in a single line
                    let has_name = name_regex.is_match(line);
                    let has_enum = enum_regex.is_match(line);

                    if has_name || has_enum {
                        used_identifier_macro = true;
                        let mut new_line = line.to_string();

                        if has_name {
                            new_line = name_regex
                                .replace_all(&new_line, |caps: &regex::Captures| {
                                    Self::expand_name(&caps[1], markdown, edition)
                                })
                                .to_string();
                        }

                        if has_enum {
                            new_line = enum_regex
                                .replace_all(&new_line, |caps: &regex::Captures| {
                                    Self::expand_enum(&caps[1], markdown, edition)
                                })
                                .to_string();
                        }

                        resulting_lines.push(new_line);
                        continue;
                    }

                    if line.starts_with("```") {
                        if in_block {
                            // This is end of block
                            // Replace previous lines
                            for block_line in block_lines.iter().cloned() {
                                let indent = std::cmp::min(min_indentation, block_line.len());
                                resulting_lines.push(block_line[indent..].to_string())
                            }
                            in_block = false;
                        } else {
                            // Start of block
                            in_block = true;
                            block_lines = vec![];
                            min_indentation = usize::MAX;
                        }

                        resulting_lines.push(line.to_string());
                        continue;
                    }

                    if in_block {
                        let line = line.replace('\t', "    ");
                        block_lines.push(line.clone());
                        let trimmed = line.trim_start_matches(' ');
                        if !trimmed.is_empty() {
                            min_indentation =
                                std::cmp::min(min_indentation, line.len() - trimmed.len())
                        }
                    } else {
                        if let Some(level) = heading_level(line) {
                            section_level = level;
                        }
                        resulting_lines.push(line.to_string());
                    }
                }

                chapter.content = resulting_lines.join("\n");
                // An edition already writes identifiers in its own language.
                if markdown && edition.is_none() && used_identifier_macro {
                    chapter.content.push_str(CASING_FOOTER);
                }
            }
        });
        Ok(book)
    }
}
