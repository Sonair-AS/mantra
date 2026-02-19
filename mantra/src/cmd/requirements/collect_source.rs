use std::path::Path;

use mantra_schema::requirements::Requirement;

use super::RequirementsError;
use crate::db::{MantraDb, RequirementChanges};
use ignore::{types::TypesBuilder, WalkBuilder};

pub async fn collect_from_source(
    db: &MantraDb,
    root: &Path,
    macro_name: &str,
) -> Result<RequirementChanges, RequirementsError> {
    let mut all_reqs = Vec::new();
    let mut all_errors = Vec::new();

    if root.is_dir() || root == Path::new("") || root == Path::new("./") {
        let root = if root == Path::new("") || root == Path::new("./") {
            std::env::current_dir().expect("Current directory must be valid.")
        } else {
            root.to_path_buf()
        };

        let walk = WalkBuilder::new(&root)
            .types(
                TypesBuilder::new()
                    .add_defaults()
                    .select("rust")
                    .build()
                    .expect("Could not create Rust file filter."),
            )
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
                if filepath.extension().is_some_and(|ext| ext == "rs") {
                    let content = std::fs::read_to_string(filepath).map_err(|_| {
                        RequirementsError::CouldNotAccessFile(filepath.display().to_string())
                    })?;

                    let origin = mantra_lang_tracing::path::make_relative(filepath, &root)
                        .unwrap_or_else(|| filepath.to_path_buf())
                        .display()
                        .to_string();

                    let result = collect_req_specs_from_rust_source(content.as_bytes(), macro_name);
                    for spec in result.specs {
                        all_reqs.push(req_spec_to_requirement(spec, &origin));
                    }
                    for error in result.errors {
                        all_errors.push(format!("{origin}: {error}"));
                    }
                }
            }
        }
    } else if root.extension().is_some_and(|ext| ext == "rs") {
        let content = std::fs::read_to_string(root)
            .map_err(|_| RequirementsError::CouldNotAccessFile(root.display().to_string()))?;

        let origin = root.display().to_string();
        let result = collect_req_specs_from_rust_source(content.as_bytes(), macro_name);
        for spec in result.specs {
            all_reqs.push(req_spec_to_requirement(spec, &origin));
        }
        for error in result.errors {
            all_errors.push(format!("{origin}: {error}"));
        }
    }

    if !all_errors.is_empty() {
        return Err(RequirementsError::InvalidReqSpecs(all_errors));
    }

    if all_reqs.is_empty() {
        log::warn!("No requirement specifications were found in source.");
        let changes = RequirementChanges {
            new_generation: db.max_req_generation().await,
            ..Default::default()
        };
        Ok(changes)
    } else {
        db.add_reqs(all_reqs)
            .await
            .map_err(RequirementsError::DbError)
    }
}

fn collect_req_specs_from_rust_source(
    src: &[u8],
    macro_name: &str,
) -> mantra_rust_trace::ReqSpecResult {
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_rust::language())
        .is_err()
    {
        return mantra_rust_trace::ReqSpecResult::default();
    }

    let tree = match parser.parse(src, None) {
        Some(t) => t,
        None => return mantra_rust_trace::ReqSpecResult::default(),
    };

    let mut result = mantra_rust_trace::ReqSpecResult::default();
    walk_tree_for_req_specs(&tree.root_node(), src, macro_name, &mut result);
    result
}

fn walk_tree_for_req_specs(
    node: &tree_sitter::Node,
    src: &[u8],
    macro_name: &str,
    result: &mut mantra_rust_trace::ReqSpecResult,
) {
    let node_result = mantra_rust_trace::collect_req_spec_from_node(node, src, macro_name);
    result.specs.extend(node_result.specs);
    result.errors.extend(node_result.errors);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_tree_for_req_specs(&child, src, macro_name, result);
    }
}

fn req_spec_to_requirement(spec: mantra_rust_trace::ReqSpec, origin: &str) -> Requirement {
    let parent = spec
        .id
        .rsplit_once('.')
        .map(|(parent, _)| parent.to_string());

    Requirement {
        id: spec.id,
        title: spec.title,
        origin: origin.to_string(),
        data: spec
            .expected_reaction
            .map(|er| serde_json::json!({"expected_reaction": er})),
        manual: false,
        deprecated: false,
        parents: parent.map(|p| vec![p]),
    }
}
