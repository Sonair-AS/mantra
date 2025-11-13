use std::collections::HashSet;
use std::path::PathBuf;

use crate::cmd::requirements::collect_common::requirements_from_content;
use crate::db::{MantraDb, RequirementChanges};
use glob::glob;
use regex::Regex;

use super::collect_common::CollectorConfig;
use super::RequirementsError;

pub async fn collect_generic(
    db: &MantraDb,
    file_globs: &Vec<String>,
    regex: &String,
    version: Option<usize>,
    ignore_verbatim: &Option<bool>,
) -> Result<RequirementChanges, RequirementsError> {
    let regex = &Regex::new(regex).expect("Could not create generic regex pattern.");

    let config = CollectorConfig {
        file_type: "generic", // Not used for generic collection.
        regex,
        track_verbatim: ignore_verbatim.unwrap_or(false),
    };

    let mut reqs = Vec::new();
    for file in &get_files_from_glob_list(file_globs) {
        println!("file: {:?}", file.display());
        // Could do this, but file access limiting factor here(?)
        // futures.push(tokio::spawn(collect_from_source(db, file, "", version, config)));
        let content = std::fs::read_to_string(file)
            .map_err(|_| RequirementsError::CouldNotAccessFile(file.display().to_string()))?;

        reqs.append(&mut requirements_from_content(
            &content,
            file.display().to_string().as_str(),
            version,
            &config,
        ));
    }

    db.add_reqs(reqs).await.map_err(RequirementsError::DbError)
}

pub fn get_files_from_glob_list(glob_list: &Vec<String>) -> Vec<PathBuf> {
    // Use a HashSet to avoid duplicates.
    let mut files = HashSet::new();
    for review in glob_list {
        for entry in glob(review).expect("Failed to read glob pattern") {
            match entry {
                Ok(path) => {
                    // println!("path: {:?}", path.display());
                    files.insert(path);
                }
                Err(e) => println!("{:?}", e),
            }
        }
    }
    files.into_iter().collect()
}
