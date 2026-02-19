use std::collections::HashMap;
use std::path::PathBuf;

use crate::db::MantraDb;

use mantra_schema::requirements::{Requirement, RequirementSchema};

mod collect_common;
mod collect_generic;
mod collect_markdown;
mod collect_source;
mod collect_typst;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum Format {
    FromWiki(WikiConfig),
    FromSchema {
        #[serde(
            alias = "filepaths",
            alias = "external-files",
            alias = "external-filepaths"
        )]
        files: Vec<PathBuf>,
    },
    FromSource(SourceReqConfig),
    FromTypst(TypstReqConfig),
    FromGeneric {
        #[serde(rename = "file-globs")]
        file_globs: Vec<String>,
        regex: String,
        #[serde(rename = "ignore-verbatim")]
        ignore_verbatim: Option<bool>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SourceReqConfig {
    #[serde(alias = "source-root")]
    pub source_root: PathBuf,
    #[serde(alias = "macro-name", default = "default_macro_name")]
    pub macro_name: String,
}

fn default_macro_name() -> String {
    "req_spec".to_string()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TypstReqConfig {
    #[serde(alias = "typst-root")]
    pub typst_root: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WikiConfig {
    #[serde(alias = "wiki-root", alias = "local-wiki-root")]
    pub root: PathBuf,
    #[serde(alias = "wiki-origin")]
    pub origin: String,
    #[serde(alias = "version", alias = "major-version")]
    pub major_version: Option<usize>,
}

#[derive(Debug, thiserror::Error)]
pub enum RequirementsError {
    #[error("Could not access file '{}'.", .0)]
    CouldNotAccessFile(String),
    #[error("{}", .0)]
    Deserialize(serde_json::Error),
    #[error("{}", .0)]
    DbError(crate::db::DbError),
    #[error("Invalid requirement definitions found:\n{}", .0.join("\n"))]
    InvalidReqSpecs(Vec<String>),
}

/// Parses all formats first, then inserts into DB in one batch.
///
/// This avoids ordering dependencies between formats (e.g., typst-defined
/// parents needed by source-defined children) and surfaces parsing errors
/// immediately without waiting for DB operations.
pub async fn collect(db: &MantraDb, formats: &[Format]) -> Result<(), RequirementsError> {
    let mut all_reqs: Vec<Requirement> = Vec::new();

    for fmt in formats {
        let reqs = match fmt {
            Format::FromWiki(wiki_cfg) => collect_markdown::parse_markdown(
                &wiki_cfg.root,
                &wiki_cfg.origin,
                wiki_cfg.major_version,
            )?,
            Format::FromSchema { files } => {
                let mut reqs = Vec::new();
                for file in files {
                    let content = tokio::fs::read_to_string(file).await.map_err(|_| {
                        RequirementsError::CouldNotAccessFile(file.display().to_string())
                    })?;
                    let schema: RequirementSchema =
                        serde_json::from_str(&content).map_err(RequirementsError::Deserialize)?;
                    reqs.extend(schema.requirements);
                }
                reqs
            }
            Format::FromSource(source_cfg) => {
                collect_source::parse_source(&source_cfg.source_root, &source_cfg.macro_name)?
            }
            Format::FromTypst(typst_cfg) => {
                collect_typst::parse_typst(&typst_cfg.typst_root)?
            }
            Format::FromGeneric {
                file_globs,
                regex,
                ignore_verbatim,
            } => collect_generic::parse_generic(file_globs, regex, None, ignore_verbatim)?,
        };

        let format_label = format_label(fmt);
        if reqs.is_empty() {
            log::warn!("No requirements found for {format_label}.");
        } else {
            println!("{}: parsed {} requirements.", format_label, reqs.len());
        }

        all_reqs.extend(reqs);
    }

    warn_duplicate_ids(&all_reqs);

    if all_reqs.is_empty() {
        println!("No requirements found across all sources.");
        return Ok(());
    }

    let changes = db
        .add_reqs(all_reqs)
        .await
        .map_err(RequirementsError::DbError)?;
    println!("{changes}");

    Ok(())
}

fn warn_duplicate_ids(reqs: &[Requirement]) {
    let mut seen: HashMap<&str, &str> = HashMap::new();
    for req in reqs {
        if let Some(prev_origin) = seen.get(req.id.as_str()) {
            if *prev_origin != req.origin {
                log::warn!(
                    "Requirement '{}' collected by multiple sources ('{}' and '{}')",
                    req.id,
                    prev_origin,
                    req.origin
                );
            }
        } else {
            seen.insert(&req.id, &req.origin);
        }
    }
}

fn format_label(fmt: &Format) -> String {
    match fmt {
        Format::FromWiki(cfg) => format!("wiki({})", cfg.root.display()),
        Format::FromSchema { files } => format!("schema({} files)", files.len()),
        Format::FromSource(cfg) => format!("source({})", cfg.source_root.display()),
        Format::FromTypst(cfg) => format!("typst({})", cfg.typst_root.display()),
        Format::FromGeneric { file_globs, .. } => {
            format!("generic({} globs)", file_globs.len())
        }
    }
}
