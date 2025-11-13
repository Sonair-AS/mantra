use std::path::PathBuf;

use crate::db::{MantraDb, RequirementChanges};

use mantra_schema::requirements::RequirementSchema;

mod collect_common;
mod collect_generic;
mod collect_markdown;

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
    FromGeneric {
        #[serde(rename = "file-globs")]
        file_globs: Vec<String>,
        regex: String,
        #[serde(rename = "ignore-verbatim")]
        ignore_verbatim: Option<bool>,
    },
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
}

pub async fn collect(db: &MantraDb, formats: &[Format]) -> Result<(), RequirementsError> {
    for fmt in formats {
        let req_changes = match fmt {
            Format::FromWiki(wiki_cfg) => {
                collect_markdown::collect_from_markdown(
                    db,
                    &wiki_cfg.root,
                    &wiki_cfg.origin,
                    wiki_cfg.major_version,
                )
                .await
            }
            Format::FromSchema { files } => {
                let mut changes = RequirementChanges::default();

                for file in files {
                    let content = tokio::fs::read_to_string(file).await.map_err(|_| {
                        RequirementsError::CouldNotAccessFile(file.display().to_string())
                    })?;
                    let schema =
                        serde_json::from_str(&content).map_err(RequirementsError::Deserialize)?;
                    changes.merge(&mut collect_from_schema(db, schema).await?);
                }

                Ok(changes)
            }
            Format::FromGeneric {
                file_globs,
                regex,
                ignore_verbatim,
            } => {
                collect_generic::collect_generic(db, file_globs, regex, None, ignore_verbatim).await
            }
        }?;
        println!("{req_changes}");
    }

    Ok(())
}

pub async fn collect_from_schema(
    db: &MantraDb,
    schema: RequirementSchema,
) -> Result<RequirementChanges, RequirementsError> {
    db.add_reqs(schema.requirements)
        .await
        .map_err(RequirementsError::DbError)
}
