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

struct SnippetsProcessor;
impl SnippetsProcessor {
    fn get_language_paths(file_base: &str) -> Vec<(&'static str, &'static str, String)> {
        vec![
            ("Rust", "rust", format!("snippets/rust/src/{}.rs", file_base)),
            ("Swift", "swift", format!("snippets/swift/BreezSdkSnippets/Sources/{}.swift", capitalize_first(file_base))),
            ("Kotlin", "kotlin", format!("snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/{}.kt", capitalize_first(file_base))),
            ("C#", "csharp", format!("snippets/csharp/{}.cs", capitalize_first(file_base))),
            ("Javascript", "typescript", format!("snippets/wasm/{}.ts", file_base)),
            ("React Native", "typescript", format!("snippets/react-native/{}.ts", file_base)),
            ("Flutter", "dart", format!("snippets/flutter/lib/{}.dart", file_base)),
            ("Python", "python", format!("snippets/python/src/{}.py", file_base)),
            ("Go", "go", format!("snippets/go/{}.go", file_base)),
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

    fn expand_tabs(
        ctx: &PreprocessorContext,
        file_base: &str,
        snippet_name: &str,
    ) -> Result<String> {
        let config = Self::get_language_paths(file_base);
        let mut result = String::from("<custom-tabs category=\"lang\">\n");

        for (lang_name, lang_code, relative_path) in &config {
            let full_path = ctx.root.join(&relative_path);

            // Try to read the file
            let content = match fs::read_to_string(&full_path) {
                Ok(content) => content,
                Err(_) => {
                    // Skip this language if file doesn't exist
                    continue;
                }
            };

            // Try to extract the snippet
            let snippet = match Self::extract_snippet(&content, snippet_name) {
                Some(snippet) => snippet,
                None => {
                    // Skip this language if snippet doesn't exist
                    continue;
                }
            };

            result.push_str(&format!(
                "<div slot=\"title\">{}</div>\n<section>\n\n```{},ignore\n{}\n```\n\n</section>\n\n",
                lang_name, lang_code, snippet
            ));
        }

        result.push_str("</custom-tabs>\n");
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
        book.for_each_mut(|item| {
            if let BookItem::Chapter(chapter) = item {
                let mut resulting_lines: Vec<String> = vec![];
                let mut in_block = false;
                let mut block_lines: Vec<String> = vec![];
                let mut min_indentation: usize = 0;

                for line in chapter.content.lines() {
                    // Check for tab expansion syntax: {{#tabs file:snippet-name}}
                    if let Some(captures) = regex::Regex::new(r"\{\{#tabs\s+([^:]+):([\w-]+)\}\}")
                        .unwrap()
                        .captures(line)
                    {
                        if let (Some(file_base), Some(snippet_name)) =
                            (captures.get(1), captures.get(2))
                        {
                            match Self::expand_tabs(ctx, file_base.as_str(), snippet_name.as_str())
                            {
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
                        resulting_lines.push(line.to_string());
                    }
                }

                chapter.content = resulting_lines.join("\n");
            }
        });
        Ok(book)
    }
}
