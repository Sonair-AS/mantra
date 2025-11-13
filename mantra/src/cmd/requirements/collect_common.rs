use std::path::Path;

use crate::db::{MantraDb, RequirementChanges};

use ignore::{types::TypesBuilder, WalkBuilder};
use mantra_schema::requirements::Requirement;
use regex::Regex;

use super::RequirementsError;

/// Configuration for collecting requirements from a specific file type
pub struct CollectorConfig<'a> {
    /// The file type to filter (e.g., "markdown", "rust")
    pub file_type: &'static str,
    /// The regex to match requirements (pre-compiled for performance)
    pub regex: &'a Regex,
    /// Whether to track verbatim/code block contexts (to skip parsing inside them)
    pub track_verbatim: bool,
}

pub async fn collect_from_source<'a>(
    db: &MantraDb,
    root: &Path,
    origin: &str,
    version: Option<usize>,
    config: CollectorConfig<'a>,
) -> Result<RequirementChanges, RequirementsError> {
    let mut reqs = Vec::new();

    if root.is_dir() {
        let walk = WalkBuilder::new(root)
            .types(
                TypesBuilder::new()
                    .add_defaults()
                    .select(config.file_type)
                    .build()
                    .unwrap_or_else(|_| {
                        panic!("Could not create {} file filter.", config.file_type)
                    }),
            )
            .build();

        for dir_entry_res in walk {
            let dir_entry = match dir_entry_res {
                Ok(entry) => entry,
                Err(_) => continue,
            };

            if dir_entry
                .file_type()
                .expect("No file type found for given entry. Note: stdin is not supported.")
                .is_file()
            {
                let content = std::fs::read_to_string(dir_entry.path()).map_err(|_| {
                    RequirementsError::CouldNotAccessFile(dir_entry.path().display().to_string())
                })?;

                let file_stem = dir_entry
                    .path()
                    .file_stem()
                    .expect("Filepath is valid filename.")
                    .to_string_lossy()
                    .replace(char::is_whitespace, "-");
                let req_origin = format!("{}/{}", origin, file_stem);

                reqs.append(&mut requirements_from_content(
                    &content,
                    &req_origin,
                    version,
                    &config,
                ));
            }
        }
    } else {
        let content = std::fs::read_to_string(root)
            .map_err(|_| RequirementsError::CouldNotAccessFile(root.display().to_string()))?;

        reqs = requirements_from_content(&content, origin, version, &config);
    }

    if reqs.is_empty() {
        log::warn!("No requirements were found.");

        let changes = RequirementChanges {
            new_generation: db.max_req_generation().await,
            ..Default::default()
        };
        Ok(changes)
    } else {
        db.add_reqs(reqs).await.map_err(RequirementsError::DbError)
    }
}

pub fn requirements_from_content(
    content: &str,
    origin: &str,
    version: Option<usize>,
    config: &CollectorConfig,
) -> Vec<Requirement> {
    let lines = content.lines();

    let mut reqs = Vec::new();
    let mut in_verbatim_context = false;

    for line in lines {
        // Track verbatim/code block contexts if needed (for markdown)
        if config.track_verbatim {
            if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
                in_verbatim_context = !in_verbatim_context;
            }
        }

        // Skip parsing if we're inside a verbatim context
        if config.track_verbatim && in_verbatim_context {
            continue;
        }

        if let Some(captures) = config.regex.captures(line) {
            let id = captures
                .name("id")
                .expect("`id` capture group was not in heading match.")
                .as_str()
                .to_string();

            let mut marker = captures.name("marker").map(|c| c.as_str().to_string());
            let extracted_version: Option<usize> = captures.name("version").map(|c| {
                c.as_str()
                    .parse()
                    .expect("Matched digits must fit into *usize*.")
            });

            if let Some(version) = version {
                if let Some(extracted_version) = extracted_version {
                    if version < extracted_version {
                        marker = None;
                    }
                }
            }

            let manual = marker == Some("manual".to_string());
            let deprecated = marker == Some("deprecated".to_string());

            let title = captures
                .name("title")
                .expect("`title` capture group was not in heading match.")
                .as_str()
                .to_string();

            reqs.push(Requirement {
                id,
                title,
                origin: origin.to_string(),
                data: None,
                manual,
                deprecated,
                parents: None,
            });
        }
    }

    reqs
}
