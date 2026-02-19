use std::str::FromStr;

use mantra_lang_tracing::collect::AstNode;

/// A requirement specification extracted from a `#<macro_name>(...)` pattern
/// in a Rust inner doc comment (`//!`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReqSpec {
    pub id: String,
    pub title: String,
    pub expected_reaction: Option<String>,
    pub line: usize,
}

/// Collects both successfully parsed specs and validation errors from a node.
#[derive(Debug, Clone, Default)]
pub struct ReqSpecResult {
    pub specs: Vec<ReqSpec>,
    pub errors: Vec<ReqSpecError>,
}

impl ReqSpecResult {
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReqSpecError {
    pub line: usize,
    pub kind: ReqSpecErrorKind,
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReqSpecErrorKind {
    /// The pattern was found in a `//` or `///` comment instead of `//!`.
    WrongCommentType,
    /// The pattern was found without the required `#` prefix.
    MissingHashPrefix,
    /// `#<macro_name>(` has no matching closing `)`.
    UnmatchedParen,
    /// `#<macro_name>()` was called with no arguments.
    EmptyArguments,
    /// Arguments inside `#<macro_name>(...)` could not be parsed as quoted strings.
    MalformedArguments,
}

impl std::fmt::Display for ReqSpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "line {}: {}\n  | {}",
            self.line, self.kind, self.context
        )
    }
}

impl std::fmt::Display for ReqSpecErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongCommentType => {
                write!(
                    f,
                    "requirement macro found in wrong comment type; use inner doc comment (//!)"
                )
            }
            Self::MissingHashPrefix => {
                write!(f, "requirement macro missing '#' prefix")
            }
            Self::UnmatchedParen => {
                write!(f, "requirement macro has no matching closing parenthesis")
            }
            Self::EmptyArguments => {
                write!(
                    f,
                    "requirement macro has empty arguments; expected at least an ID"
                )
            }
            Self::MalformedArguments => {
                write!(
                    f,
                    "requirement macro has malformed arguments; expected quoted strings like #macro(\"ID\", \"title\")"
                )
            }
        }
    }
}

impl std::error::Error for ReqSpecError {}

/// Inspects a single AST node for requirement patterns and validates them.
///
/// `macro_name` is the name to look for (e.g., `"req_spec"`). The function
/// searches for `#<macro_name>(...)` inside inner doc comments (`//!`).
///
/// Any `line_comment` node containing `<macro_name>(` is examined:
/// - If it's not an inner doc comment (`//!`), an error is reported.
/// - If the `#` prefix is missing, an error is reported.
/// - If parentheses are unmatched or arguments malformed, an error is reported.
/// - Valid patterns produce `ReqSpec` entries.
pub fn collect_req_spec_from_node(
    node: &AstNode,
    src: &[u8],
    macro_name: &str,
) -> ReqSpecResult {
    let mut result = ReqSpecResult::default();

    if node.kind() != "line_comment" {
        return result;
    }

    let text = match node.utf8_text(src) {
        Ok(t) => t,
        Err(_) => return result,
    };

    let search_pattern = format!("{macro_name}(");
    if !text.contains(&search_pattern) {
        return result;
    }

    let line = node.start_position().row + 1;

    if !text.starts_with("//!") {
        result.errors.push(ReqSpecError {
            line,
            kind: ReqSpecErrorKind::WrongCommentType,
            context: text.to_string(),
        });
        return result;
    }

    let content = &text[3..];

    let mut search_from = 0;
    while let Some(pos) = content[search_from..].find(&search_pattern) {
        let abs_pos = search_from + pos;

        if abs_pos == 0 || content.as_bytes()[abs_pos - 1] != b'#' {
            result.errors.push(ReqSpecError {
                line,
                kind: ReqSpecErrorKind::MissingHashPrefix,
                context: text.to_string(),
            });
            search_from = abs_pos + search_pattern.len();
            continue;
        }

        let paren_content_start = abs_pos + search_pattern.len();

        match find_matching_paren(content, paren_content_start) {
            Some(paren_end) => {
                let args_str = &content[paren_content_start..paren_end];
                if args_str.trim().is_empty() {
                    result.errors.push(ReqSpecError {
                        line,
                        kind: ReqSpecErrorKind::EmptyArguments,
                        context: text.to_string(),
                    });
                } else {
                    match parse_req_spec_args(args_str, line) {
                        Some(spec) => result.specs.push(spec),
                        None => result.errors.push(ReqSpecError {
                            line,
                            kind: ReqSpecErrorKind::MalformedArguments,
                            context: text.to_string(),
                        }),
                    }
                }
                search_from = paren_end + 1;
            }
            None => {
                result.errors.push(ReqSpecError {
                    line,
                    kind: ReqSpecErrorKind::UnmatchedParen,
                    context: text.to_string(),
                });
                break;
            }
        }
    }

    result
}

/// Finds the index of the matching closing parenthesis.
/// `start` is the index of the first character after the opening paren.
pub fn find_matching_paren(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth: u32 = 1;
    let mut i = start;
    let mut in_string = false;
    let mut escape_next = false;

    while i < bytes.len() {
        let ch = bytes[i];

        if escape_next {
            escape_next = false;
            i += 1;
            continue;
        }

        if in_string {
            match ch {
                b'\\' => escape_next = true,
                b'"' => in_string = false,
                _ => {}
            }
        } else {
            match ch {
                b'"' => in_string = true,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }

        i += 1;
    }

    None
}

/// Parses comma-separated quoted string arguments from a parenthesized expression.
///
/// Returns the extracted strings, or `None` if the input contains non-string,
/// non-comma tokens or no strings at all.
pub fn parse_quoted_args(args_str: &str) -> Option<Vec<String>> {
    let tokens = proc_macro2::TokenStream::from_str(args_str).ok()?;
    let mut string_args: Vec<String> = Vec::new();

    for token in tokens {
        match token {
            proc_macro2::TokenTree::Literal(lit) => {
                let s = lit.to_string();
                let unquoted = s
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .unwrap_or(&s)
                    .to_string();
                string_args.push(unquoted);
            }
            proc_macro2::TokenTree::Punct(p) if p.as_char() == ',' => {}
            _ => return None,
        }
    }

    if string_args.is_empty() {
        return None;
    }

    Some(string_args)
}

fn parse_req_spec_args(args_str: &str, line: usize) -> Option<ReqSpec> {
    let mut string_args = parse_quoted_args(args_str)?;

    let id = string_args.remove(0);
    let title = if !string_args.is_empty() {
        string_args.remove(0)
    } else {
        String::new()
    };
    let expected_reaction = if !string_args.is_empty() {
        Some(string_args.remove(0))
    } else {
        None
    };

    Some(ReqSpec {
        id,
        title,
        expected_reaction,
        line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_MACRO: &str = "req_spec";

    fn parse_all(src: &str) -> ReqSpecResult {
        parse_all_with_macro(src, DEFAULT_MACRO)
    }

    fn parse_all_with_macro(src: &str, macro_name: &str) -> ReqSpecResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::language())
            .unwrap();
        let tree = parser.parse(src.as_bytes(), None).unwrap();

        let mut result = ReqSpecResult::default();
        collect_from_tree(&tree.root_node(), src.as_bytes(), macro_name, &mut result);
        result
    }

    fn collect_from_tree(
        node: &AstNode,
        src: &[u8],
        macro_name: &str,
        result: &mut ReqSpecResult,
    ) {
        let node_result = collect_req_spec_from_node(node, src, macro_name);
        result.specs.extend(node_result.specs);
        result.errors.extend(node_result.errors);

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_from_tree(&child, src, macro_name, result);
        }
    }

    // --- Happy path tests ---

    #[test]
    fn test_should_extract_req_spec_with_id_and_title() {
        let src = r#"
#[cfg(test)]
mod tests {
    //! * #req_spec("MY.ID", "My title")
}
"#;
        let result = parse_all(src);
        assert!(
            !result.has_errors(),
            "Unexpected errors: {:?}",
            result.errors
        );
        assert_eq!(result.specs.len(), 1);
        assert_eq!(result.specs[0].id, "MY.ID");
        assert_eq!(result.specs[0].title, "My title");
        assert_eq!(result.specs[0].expected_reaction, None);
    }

    #[test]
    fn test_should_extract_req_spec_with_expected_reaction() {
        let src = r#"
#[cfg(test)]
mod tests {
    //! * #req_spec("MY.ID.C1", "Some condition", "Returns true")
}
"#;
        let result = parse_all(src);
        assert!(
            !result.has_errors(),
            "Unexpected errors: {:?}",
            result.errors
        );
        assert_eq!(result.specs.len(), 1);
        assert_eq!(result.specs[0].id, "MY.ID.C1");
        assert_eq!(result.specs[0].title, "Some condition");
        assert_eq!(
            result.specs[0].expected_reaction,
            Some("Returns true".to_string())
        );
    }

    #[test]
    fn test_should_extract_multiple_req_specs_from_separate_lines() {
        let src = r#"
#[cfg(test)]
mod tests {
    //! * #req_spec("MY.ID", "Parent spec")
    //! * #req_spec("MY.ID.C1", "Condition 1", "Returns true")
    //! * #req_spec("MY.ID.C2", "Condition 2", "Returns false")
}
"#;
        let result = parse_all(src);
        assert!(
            !result.has_errors(),
            "Unexpected errors: {:?}",
            result.errors
        );
        assert_eq!(result.specs.len(), 3);
        assert_eq!(result.specs[0].id, "MY.ID");
        assert_eq!(result.specs[1].id, "MY.ID.C1");
        assert_eq!(result.specs[2].id, "MY.ID.C2");
    }

    #[test]
    fn test_should_extract_req_spec_with_only_id() {
        let src = r#"
#[cfg(test)]
mod tests {
    //! * #req_spec("MY.ID")
}
"#;
        let result = parse_all(src);
        assert!(
            !result.has_errors(),
            "Unexpected errors: {:?}",
            result.errors
        );
        assert_eq!(result.specs.len(), 1);
        assert_eq!(result.specs[0].id, "MY.ID");
        assert_eq!(result.specs[0].title, "");
        assert_eq!(result.specs[0].expected_reaction, None);
    }

    #[test]
    fn test_should_report_correct_line_numbers() {
        let src = r#"
#[cfg(test)]
mod tests {
    //! * #req_spec("FIRST", "First spec")
    //! Some other doc text
    //! * #req_spec("SECOND", "Second spec")
}
"#;
        let result = parse_all(src);
        assert!(
            !result.has_errors(),
            "Unexpected errors: {:?}",
            result.errors
        );
        assert_eq!(result.specs.len(), 2);
        assert_eq!(result.specs[0].line, 4);
        assert_eq!(result.specs[1].line, 6);
    }

    #[test]
    fn test_should_handle_realistic_test_module() {
        let src = r#"
#[cfg(test)]
mod test {
    //! Boundary value analysis and equivalence classes
    //!
    //! # Equivalence classes for JammerDetector
    //!
    //! * #req_spec("P0.LIB.PROCESS.PREPROCESS.JAMMERDETECTOR", "Test specs for jammer_detection::JammerDetector")
    //! * #req_spec("P0.LIB.PROCESS.PREPROCESS.JAMMERDETECTOR.UT.RAWDATA", "Rawdata interface")
    //! * #req_spec("P0.LIB.PROCESS.PREPROCESS.JAMMERDETECTOR.UT.RAWDATA.C1", "No data above threshold", "Returns false")
    //! * #req_spec("P0.LIB.PROCESS.PREPROCESS.JAMMERDETECTOR.UT.RAWDATA.C2", "Only data in direct sound range above threshold", "Returns false")
    //! * #req_spec("P0.LIB.PROCESS.PREPROCESS.JAMMERDETECTOR.UT.RAWDATA.C3", "All data above threshold", "Returns true")
    use mantra_rust_macros::req;

    #[test]
    #[req("P0.LIB.PROCESS.PREPROCESS.JAMMERDETECTOR.UT.RAWDATA.C1")]
    fn test_no_data_above_threshold() {}
}
"#;
        let result = parse_all(src);
        assert!(
            !result.has_errors(),
            "Unexpected errors: {:?}",
            result.errors
        );
        assert_eq!(result.specs.len(), 5);
        assert_eq!(
            result.specs[0].id,
            "P0.LIB.PROCESS.PREPROCESS.JAMMERDETECTOR"
        );
        assert_eq!(
            result.specs[0].title,
            "Test specs for jammer_detection::JammerDetector"
        );
        assert_eq!(result.specs[0].expected_reaction, None);

        assert_eq!(
            result.specs[2].id,
            "P0.LIB.PROCESS.PREPROCESS.JAMMERDETECTOR.UT.RAWDATA.C1"
        );
        assert_eq!(result.specs[2].title, "No data above threshold");
        assert_eq!(
            result.specs[2].expected_reaction,
            Some("Returns false".to_string())
        );
    }

    #[test]
    fn test_should_not_flag_inner_doc_comments_without_req_spec() {
        let src = r#"
#[cfg(test)]
mod tests {
    //! This module tests the FooBar component.
    //! It uses default parameters.
}
"#;
        let result = parse_all(src);
        assert!(result.specs.is_empty());
        assert!(!result.has_errors());
    }

    // --- Custom macro name tests ---

    #[test]
    fn test_should_work_with_custom_macro_name() {
        let src = r#"
#[cfg(test)]
mod tests {
    //! * #my_req("MY.ID", "Custom macro name")
}
"#;
        let result = parse_all_with_macro(src, "my_req");
        assert!(
            !result.has_errors(),
            "Unexpected errors: {:?}",
            result.errors
        );
        assert_eq!(result.specs.len(), 1);
        assert_eq!(result.specs[0].id, "MY.ID");
        assert_eq!(result.specs[0].title, "Custom macro name");
    }

    #[test]
    fn test_should_not_match_different_macro_name() {
        let src = r#"
#[cfg(test)]
mod tests {
    //! * #req_spec("MY.ID", "Should not match")
}
"#;
        let result = parse_all_with_macro(src, "other_macro");
        assert!(result.specs.is_empty());
        assert!(!result.has_errors());
    }

    // --- Error detection tests ---

    #[test]
    fn test_should_error_on_req_spec_in_regular_comment() {
        let src = r#"
// #req_spec("MY.ID", "Wrong comment type")
fn some_function() {}
"#;
        let result = parse_all(src);
        assert!(result.specs.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].kind, ReqSpecErrorKind::WrongCommentType);
        assert_eq!(result.errors[0].line, 2);
    }

    #[test]
    fn test_should_error_on_req_spec_in_outer_doc_comment() {
        let src = r#"
/// #req_spec("MY.ID", "Wrong comment type")
fn some_function() {}
"#;
        let result = parse_all(src);
        assert!(result.specs.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].kind, ReqSpecErrorKind::WrongCommentType);
    }

    #[test]
    fn test_should_error_on_missing_hash_prefix() {
        let src = r#"
#[cfg(test)]
mod tests {
    //! * req_spec("MY.ID", "Missing hash")
}
"#;
        let result = parse_all(src);
        assert!(result.specs.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].kind, ReqSpecErrorKind::MissingHashPrefix);
        assert_eq!(result.errors[0].line, 4);
    }

    #[test]
    fn test_should_error_on_unmatched_paren() {
        let src = r#"
#[cfg(test)]
mod tests {
    //! * #req_spec("MY.ID", "Unclosed paren
}
"#;
        let result = parse_all(src);
        assert!(result.specs.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].kind, ReqSpecErrorKind::UnmatchedParen);
    }

    #[test]
    fn test_should_error_on_empty_arguments() {
        let src = r#"
#[cfg(test)]
mod tests {
    //! * #req_spec()
}
"#;
        let result = parse_all(src);
        assert!(result.specs.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].kind, ReqSpecErrorKind::EmptyArguments);
    }

    #[test]
    fn test_should_error_on_malformed_arguments() {
        let src = r#"
#[cfg(test)]
mod tests {
    //! * #req_spec(MY.ID, "title")
}
"#;
        let result = parse_all(src);
        assert!(result.specs.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].kind, ReqSpecErrorKind::MalformedArguments);
    }

    #[test]
    fn test_should_collect_errors_from_multiple_lines() {
        let src = r#"
#[cfg(test)]
mod tests {
    //! * #req_spec("VALID.ID", "This one is fine")
    //! * req_spec("BAD.ONE", "Missing hash")
    //! * #req_spec()
}
"#;
        let result = parse_all(src);
        assert_eq!(result.specs.len(), 1);
        assert_eq!(result.specs[0].id, "VALID.ID");
        assert_eq!(result.errors.len(), 2);
        assert_eq!(result.errors[0].kind, ReqSpecErrorKind::MissingHashPrefix);
        assert_eq!(result.errors[1].kind, ReqSpecErrorKind::EmptyArguments);
    }

    #[test]
    fn test_should_error_on_custom_macro_in_wrong_comment() {
        let src = r#"
// #my_req("MY.ID", "Wrong comment type")
fn some_function() {}
"#;
        let result = parse_all_with_macro(src, "my_req");
        assert!(result.specs.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].kind, ReqSpecErrorKind::WrongCommentType);
    }

    #[test]
    fn test_error_display_includes_line_and_context() {
        let error = ReqSpecError {
            line: 42,
            kind: ReqSpecErrorKind::WrongCommentType,
            context: "// #req_spec(\"MY.ID\", \"title\")".to_string(),
        };
        let display = error.to_string();
        assert!(display.contains("line 42"));
        assert!(display.contains("wrong comment type"));
        assert!(display.contains("// #req_spec"));
    }

    // --- find_matching_paren unit tests ---

    #[test]
    fn test_find_matching_paren_simple() {
        assert_eq!(find_matching_paren("abc)", 0), Some(3));
    }

    #[test]
    fn test_find_matching_paren_nested() {
        assert_eq!(find_matching_paren("a(b)c)", 0), Some(5));
    }

    #[test]
    fn test_find_matching_paren_in_string() {
        assert_eq!(find_matching_paren(r#""a)b")"#, 0), Some(5));
    }

    #[test]
    fn test_find_matching_paren_unmatched() {
        assert_eq!(find_matching_paren("abc", 0), None);
    }
}
