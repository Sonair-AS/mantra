use std::path::Path;

use mantra_schema::requirements::Requirement;

use super::RequirementsError;
use ignore::{types::TypesBuilder, WalkBuilder};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypstReqError {
    pub line: usize,
    pub kind: TypstReqErrorKind,
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypstReqErrorKind {
    UnmatchedParen,
    EmptyArguments,
    MalformedArguments,
}

impl std::fmt::Display for TypstReqError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "line {}: {}\n  | {}",
            self.line, self.kind, self.context
        )
    }
}

impl std::fmt::Display for TypstReqErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnmatchedParen => {
                write!(f, "requirement call has no matching closing parenthesis")
            }
            Self::EmptyArguments => {
                write!(
                    f,
                    "requirement call has empty arguments; expected at least an ID"
                )
            }
            Self::MalformedArguments => {
                write!(
                    f,
                    "requirement call has malformed arguments; expected quoted strings like req(\"ID\", \"title\")"
                )
            }
        }
    }
}

pub fn parse_typst(
    root: &Path,
) -> Result<Vec<Requirement>, RequirementsError> {
    let mut all_reqs = Vec::new();
    let mut all_errors = Vec::new();

    let walk_root = if root == Path::new("") || root == Path::new("./") {
        std::env::current_dir().expect("Current directory must be valid.")
    } else {
        root.to_path_buf()
    };

    if walk_root.is_dir() {
        let mut types_builder = TypesBuilder::new();
        types_builder
            .add("typst", "*.typ")
            .expect("Valid typst glob pattern");
        types_builder.select("typst");

        let walk = WalkBuilder::new(&walk_root)
            .types(types_builder.build().expect("Could not create typst file filter."))
            .build();

        for dir_entry_res in walk {
            let dir_entry = match dir_entry_res {
                Ok(entry) => entry,
                Err(_) => continue,
            };

            if dir_entry
                .file_type()
                .map(|ft| ft.is_file())
                .unwrap_or(false)
            {
                let filepath = dir_entry.path();
                if filepath.extension().is_some_and(|ext| ext == "typ") {
                    let content = std::fs::read_to_string(filepath).map_err(|_| {
                        RequirementsError::CouldNotAccessFile(filepath.display().to_string())
                    })?;

                    let origin = mantra_lang_tracing::path::make_relative(filepath, &walk_root)
                        .unwrap_or_else(|| filepath.to_path_buf())
                        .display()
                        .to_string();

                    let (reqs, errors) = extract_reqs_from_typst(&content, &origin);
                    all_reqs.extend(reqs);
                    for error in errors {
                        all_errors.push(format!("{origin}: {error}"));
                    }
                }
            }
        }
    } else if root.extension().is_some_and(|ext| ext == "typ") {
        let content = std::fs::read_to_string(root)
            .map_err(|_| RequirementsError::CouldNotAccessFile(root.display().to_string()))?;

        let origin = root.display().to_string();
        let (reqs, errors) = extract_reqs_from_typst(&content, &origin);
        all_reqs.extend(reqs);
        for error in errors {
            all_errors.push(format!("{origin}: {error}"));
        }
    }

    if !all_errors.is_empty() {
        return Err(RequirementsError::InvalidReqSpecs(all_errors));
    }

    Ok(all_reqs)
}

fn extract_reqs_from_typst(
    content: &str,
    origin: &str,
) -> (Vec<Requirement>, Vec<TypstReqError>) {
    let mut reqs = Vec::new();
    let mut errors = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        let line_number = line_idx + 1;
        let matches = find_req_calls(line);

        for (args_start_in_line, _macro_name) in matches {
            match mantra_rust_trace::find_matching_paren(line, args_start_in_line) {
                Some(paren_end) => {
                    let args_str = &line[args_start_in_line..paren_end];
                    if args_str.trim().is_empty() {
                        errors.push(TypstReqError {
                            line: line_number,
                            kind: TypstReqErrorKind::EmptyArguments,
                            context: line.trim().to_string(),
                        });
                    } else {
                        match parse_typst_req_args(args_str) {
                            Some((id, title)) => {
                                let parent = id
                                    .rsplit_once('.')
                                    .map(|(parent, _)| parent.to_string());

                                reqs.push(Requirement {
                                    id,
                                    title,
                                    origin: origin.to_string(),
                                    data: None,
                                    manual: false,
                                    deprecated: false,
                                    parents: parent.map(|p| vec![p]),
                                });
                            }
                            None => {
                                errors.push(TypstReqError {
                                    line: line_number,
                                    kind: TypstReqErrorKind::MalformedArguments,
                                    context: line.trim().to_string(),
                                });
                            }
                        }
                    }
                }
                None => {
                    errors.push(TypstReqError {
                        line: line_number,
                        kind: TypstReqErrorKind::UnmatchedParen,
                        context: line.trim().to_string(),
                    });
                }
            }
        }
    }

    (reqs, errors)
}

/// Parses arguments of a `req(...)` or `reqt(...)` call.
///
/// Supports two title formats:
/// - Quoted string: `"ID", "title"` or `"ID", "title", error("Type")`
/// - Typst content block: `"ID", [title with #link("url")[display]]`
///
/// The ID (first argument) must always be a quoted string.
/// Any arguments beyond the second (e.g., `error("Type")`, `panic`) are ignored.
/// Returns `(id, title)` on success.
fn parse_typst_req_args(args_str: &str) -> Option<(String, String)> {
    // Try all-quoted first (most common case, and handles 3+ quoted args)
    if let Some(mut args) = mantra_rust_trace::parse_quoted_args(args_str) {
        if !args.is_empty() {
            let id = args.remove(0);
            let title = if !args.is_empty() {
                args.remove(0)
            } else {
                String::new()
            };
            return Some((id, title));
        }
    }

    // Manual parsing for mixed formats (content blocks, or non-quoted third args
    // like error("Type") and panic)
    let trimmed = args_str.trim();
    if !trimmed.starts_with('"') {
        return None;
    }

    let id_end = trimmed[1..].find('"')? + 1;
    let id = trimmed[1..id_end].to_string();
    if id.is_empty() {
        return None;
    }

    let rest = trimmed[id_end + 1..].trim();
    if rest.is_empty() {
        return Some((id, String::new()));
    }

    let rest = rest.strip_prefix(',')?.trim();
    if rest.is_empty() {
        return Some((id, String::new()));
    }

    if rest.starts_with('[') {
        let title = extract_content_block(rest)?;
        Some((id, title))
    } else if rest.starts_with('"') {
        let title_end = rest[1..].find('"')? + 1;
        let title = rest[1..title_end].to_string();
        Some((id, title))
    } else {
        None
    }
}

/// Extracts the text inside a balanced Typst content block `[...]`.
///
/// Handles nested brackets and skips brackets inside quoted strings.
fn extract_content_block(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'[') {
        return None;
    }

    let mut depth: usize = 0;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[1..i].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }

    None
}

/// Finds `#req(`, `req(`, `#reqt(`, and `reqt(` calls in a line.
///
/// Returns `(args_start, macro_name)` where `args_start` is the byte offset
/// immediately after the opening parenthesis.
///
/// Does NOT match `req_table(`, `req_entry(`, `req_spec(`, etc.
fn find_req_calls(line: &str) -> Vec<(usize, &'static str)> {
    let mut results = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'"' {
            i += 1;
            while i < len && bytes[i] != b'"' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }

        if i + 4 <= len && &bytes[i..i + 4] == b"reqt" {
            let preceded_by_ident = i > 0
                && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let followed_by_paren = i + 4 < len && bytes[i + 4] == b'(';
            let followed_by_ident = i + 4 < len
                && (bytes[i + 4].is_ascii_alphanumeric() || bytes[i + 4] == b'_');

            if !preceded_by_ident && followed_by_paren && !followed_by_ident {
                results.push((i + 5, "reqt"));
                i += 5;
                continue;
            }
        }

        if i + 3 <= len && &bytes[i..i + 3] == b"req" {
            let preceded_by_ident = i > 0
                && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let followed_by_paren = i + 3 < len && bytes[i + 3] == b'(';
            let followed_by_ident = i + 3 < len
                && (bytes[i + 3].is_ascii_alphanumeric() || bytes[i + 3] == b'_');

            if !preceded_by_ident && followed_by_paren && !followed_by_ident {
                results.push((i + 4, "req"));
                i += 4;
                continue;
            }
        }

        i += 1;
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- find_req_calls tests ---

    #[test]
    fn test_should_find_hash_req() {
        let calls = find_req_calls(r#"#req("SOME.ID", "title")"#);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "req");
    }

    #[test]
    fn test_should_find_reqt() {
        let calls = find_req_calls(r#"    reqt("SOME.ID", "title"),"#);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "reqt");
    }

    #[test]
    fn test_should_not_match_req_table() {
        let calls = find_req_calls(r#"#req_table("Name", [description])"#);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_should_not_match_req_entry() {
        let calls = find_req_calls(r#"  ..req_entry("#);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_should_not_match_req_spec() {
        let calls = find_req_calls(r#"#req_spec("ID", "title")"#);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_should_find_both_req_and_reqt_in_same_line() {
        let calls = find_req_calls(r#"#req("A", "a") reqt("B", "b")"#);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, "req");
        assert_eq!(calls[1].1, "reqt");
    }

    #[test]
    fn test_should_not_match_req_inside_quoted_string() {
        let calls = find_req_calls(r#"some_func("contains req(ID) text")"#);
        assert!(calls.is_empty());
    }

    // --- extract_reqs_from_typst tests ---

    #[test]
    fn test_should_extract_simple_req() {
        let content = r#"#req("MY.ID", "My title")"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(errors.is_empty(), "Unexpected errors: {:?}", errors);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].id, "MY.ID");
        assert_eq!(reqs[0].title, "My title");
    }

    #[test]
    fn test_should_extract_reqt() {
        let content = r#"    reqt("MY.ID.SUB", "A sub requirement"),"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(errors.is_empty(), "Unexpected errors: {:?}", errors);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].id, "MY.ID.SUB");
        assert_eq!(reqs[0].title, "A sub requirement");
    }

    #[test]
    fn test_should_derive_parent_from_id() {
        let content = r#"#req("PARENT.CHILD", "Title")"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(errors.is_empty());
        assert_eq!(reqs[0].parents, Some(vec!["PARENT".to_string()]));
    }

    #[test]
    fn test_should_extract_multiple_reqs_from_content() {
        let content = r#"
DeviceState returns the current device state.
#req("P0.GET_STATE", "Gets the current device state")

SetDeviceState:
#req("P0.SET_STATE", "Sets the device state")
#req("P0.SET_STATE.ENABLED", "Set enabled state")
#req("P0.SET_STATE.DISABLED", "Set disabled state")
"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(errors.is_empty(), "Unexpected errors: {:?}", errors);
        assert_eq!(reqs.len(), 4);
        assert_eq!(reqs[0].id, "P0.GET_STATE");
        assert_eq!(reqs[1].id, "P0.SET_STATE");
        assert_eq!(reqs[2].id, "P0.SET_STATE.ENABLED");
        assert_eq!(reqs[3].id, "P0.SET_STATE.DISABLED");
    }

    #[test]
    fn test_should_extract_from_req_table_pattern() {
        let content = r#"
#req_table(
  "PingTimer",
  [PingTimer description],
  ..req_entry(
    reqt("P0.PINGTIMER.SW.OK_NO_DELAY", "Wait ok, no delay"),
    "PingTimer waits for the next ping time",
    test_spec(
      "Prepare the PingTimer mock",
      "Call WaitNextSync",
      "PingTimer returns Ok(0)",
    ),
  ),
  ..req_entry(
    reqt("P0.PINGTIMER.SW.OK_DELAYED", "Wait ok, delayed"),
    "PingTimer waits delayed",
    test_spec(
      "Prepare the PingTimer mock",
      "Call WaitNextSync",
      "PingTimer returns Ok(5ms)",
    ),
  ),
)
"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(errors.is_empty(), "Unexpected errors: {:?}", errors);
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].id, "P0.PINGTIMER.SW.OK_NO_DELAY");
        assert_eq!(reqs[0].title, "Wait ok, no delay");
        assert_eq!(reqs[1].id, "P0.PINGTIMER.SW.OK_DELAYED");
        assert_eq!(reqs[1].title, "Wait ok, delayed");
    }

    #[test]
    fn test_should_error_on_unmatched_paren() {
        let content = r#"#req("MY.ID", "Unclosed"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(reqs.is_empty());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, TypstReqErrorKind::UnmatchedParen);
    }

    #[test]
    fn test_should_error_on_empty_arguments() {
        let content = r#"#req()"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(reqs.is_empty());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, TypstReqErrorKind::EmptyArguments);
    }

    #[test]
    fn test_should_error_on_malformed_arguments() {
        let content = r#"#req(MY.ID, "title")"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(reqs.is_empty());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, TypstReqErrorKind::MalformedArguments);
    }

    #[test]
    fn test_should_set_correct_origin() {
        let content = r#"#req("MY.ID", "Title")"#;
        let (reqs, _) = extract_reqs_from_typst(content, "specs/my_file.typ");
        assert_eq!(reqs[0].origin, "specs/my_file.typ");
    }

    #[test]
    fn test_should_handle_mixed_req_and_reqt() {
        let content = r#"
#req("A.TOP", "Top-level requirement")

#req_table(
  "MyTable",
  [Description],
  ..req_entry(
    reqt("A.TOP.SUB1", "Sub requirement 1"),
    "desc",
  ),
)
"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(errors.is_empty(), "Unexpected errors: {:?}", errors);
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].id, "A.TOP");
        assert_eq!(reqs[1].id, "A.TOP.SUB1");
    }

    #[test]
    fn test_should_handle_leading_space_in_title() {
        let content = r#"reqt("MY.ID", " Leading space title")"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(errors.is_empty());
        assert_eq!(reqs[0].title, " Leading space title");
    }

    // --- Content block tests ---

    #[test]
    fn test_should_extract_content_block_title() {
        let content = r#"#req("A.B.INVALID", [Value outside limits])"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(errors.is_empty(), "Unexpected errors: {:?}", errors);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].id, "A.B.INVALID");
        assert_eq!(reqs[0].title, "Value outside limits");
    }

    #[test]
    fn test_should_extract_content_block_with_nested_brackets() {
        let content = r#"#req("A.B.INVALID", [Description with #link("https://example.com/issue-42")[ISSUE-42]])"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(errors.is_empty(), "Unexpected errors: {:?}", errors);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].id, "A.B.INVALID");
        assert_eq!(
            reqs[0].title,
            r#"Description with #link("https://example.com/issue-42")[ISSUE-42]"#
        );
    }

    #[test]
    fn test_should_handle_mixed_quoted_and_content_block() {
        let content = r#"
#req("A.B", "Top-level description")
#req("A.B.VALID", "Normal case")
#req("A.B.INVALID", [Error case. See #link("https://example.com")[details]])
"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(errors.is_empty(), "Unexpected errors: {:?}", errors);
        assert_eq!(reqs.len(), 3);
        assert_eq!(reqs[0].title, "Top-level description");
        assert_eq!(reqs[1].title, "Normal case");
        assert_eq!(
            reqs[2].title,
            r#"Error case. See #link("https://example.com")[details]"#
        );
    }

    #[test]
    fn test_should_handle_error_as_third_arg() {
        let content = r#"#req("A.B.S0", "Value below threshold", error("InvalidConfig"))"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(errors.is_empty(), "Unexpected errors: {:?}", errors);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].id, "A.B.S0");
        assert_eq!(reqs[0].title, "Value below threshold");
    }

    #[test]
    fn test_should_handle_panic_as_third_arg() {
        let content = r#"#req("A.B.S1", "Corrupted state detected", panic)"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(errors.is_empty(), "Unexpected errors: {:?}", errors);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].id, "A.B.S1");
        assert_eq!(reqs[0].title, "Corrupted state detected");
    }

    #[test]
    fn test_should_handle_content_block_with_error_third_arg() {
        let content = r#"#req("A.B.S2", [Value outside limits], error("OutOfRange"))"#;
        let (reqs, errors) = extract_reqs_from_typst(content, "test.typ");
        assert!(errors.is_empty(), "Unexpected errors: {:?}", errors);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].id, "A.B.S2");
        assert_eq!(reqs[0].title, "Value outside limits");
    }

    #[test]
    fn test_should_extract_content_block_helper() {
        assert_eq!(
            extract_content_block("[simple text]"),
            Some("simple text".to_string())
        );
        assert_eq!(
            extract_content_block("[nested [brackets] here]"),
            Some("nested [brackets] here".to_string())
        );
        assert_eq!(extract_content_block("[unmatched"), None);
        assert_eq!(extract_content_block("not a block"), None);
    }
}
