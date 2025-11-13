use std::path::Path;

use crate::db::{MantraDb, RequirementChanges};
use regex::Regex;

use super::collect_common::{collect_from_source, CollectorConfig};
use super::RequirementsError;

const MARKDOWN_REGEX_PATTERN: &str = r"^(?:#{1,6}|\||\*).*?`(?<id>[^\s:]+)`(?:\((?:v(?<version>\d{1,7}):)?(?<marker>[^\)]+)\))?:\s+(?<title>.*?)(?:\s*\||$)";

static MARKDOWN_REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();

pub async fn collect_from_markdown(
    db: &MantraDb,
    root: &Path,
    origin: &str,
    version: Option<usize>,
) -> Result<RequirementChanges, RequirementsError> {
    let regex = MARKDOWN_REGEX.get_or_init(|| {
        Regex::new(MARKDOWN_REGEX_PATTERN).expect("Could not create markdown regex pattern.")
    });

    let config = CollectorConfig {
        file_type: "markdown",
        regex,
        track_verbatim: true,
    };

    collect_from_source(db, root, origin, version, config).await
}

#[cfg(test)]
mod tests {
    use super::super::collect_common::requirements_from_content;
    use super::*;

    #[test]
    fn test_requirements_from_markdown_content() {
        let content = "# `req_id`: Requirement title";
        let regex = MARKDOWN_REGEX.get_or_init(|| {
            Regex::new(MARKDOWN_REGEX_PATTERN).expect("Could not create markdown regex pattern.")
        });
        let config = CollectorConfig {
            file_type: "markdown",
            regex,
            track_verbatim: true,
        };
        let reqs = requirements_from_content(&content, "local", None, &config);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].id, "req_id");
        assert_eq!(reqs[0].title, "Requirement title");
    }

    #[test]
    fn test_requirements_from_markdown_content_with_table() {
        let content = "
# `req_header`: Requirement header
| `req_table_id`: Requirement table | Some text |
| Not first column | `req_table_id_w_marker`(v12:manual): Requirement table | Some text |
        ";
        let regex = MARKDOWN_REGEX.get_or_init(|| {
            Regex::new(MARKDOWN_REGEX_PATTERN).expect("Could not create markdown regex pattern.")
        });
        let config = CollectorConfig {
            file_type: "markdown",
            regex,
            track_verbatim: true,
        };
        let reqs = requirements_from_content(&content, "local", None, &config);
        assert_eq!(reqs.len(), 3);
        assert_eq!(reqs[0].id, "req_header");
        assert_eq!(reqs[0].title, "Requirement header");
        assert_eq!(reqs[1].id, "req_table_id");
        assert_eq!(reqs[1].title, "Requirement table");
        assert_eq!(reqs[1].manual, false);
        assert_eq!(reqs[2].id, "req_table_id_w_marker");
        assert_eq!(reqs[2].title, "Requirement table");
        assert_eq!(reqs[2].manual, true);
    }
}
