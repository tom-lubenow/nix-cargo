use std::collections::HashMap;

use crate::model::Plan;

pub fn workspace_source_prefixes_by_package(plan: &Plan) -> HashMap<String, Vec<String>> {
    plan.packages
        .iter()
        .map(|package| {
            let prefixes = local_workspace_source_prefix(&plan.workspace_root, &package.source)
                .map(|prefix| vec![prefix])
                .unwrap_or_default();
            (package.key.clone(), prefixes)
        })
        .collect()
}

fn local_workspace_source_prefix(workspace_root: &str, source: &str) -> Option<String> {
    let workspace_root = workspace_root.trim_end_matches('/');
    let source = source
        .strip_prefix("path+file://")
        .or_else(|| source.strip_prefix("path+file:"))
        .unwrap_or(source);
    let source = source.split_once('?').map(|(value, _)| value).unwrap_or(source);
    let source = source.split_once('#').map(|(value, _)| value).unwrap_or(source);
    let source = source.trim_end_matches('/');
    if source == workspace_root {
        return Some(String::new());
    }
    let prefix = format!("{workspace_root}/");
    let rel = source.strip_prefix(&prefix)?;
    if rel.is_empty() {
        None
    } else {
        Some(rel.to_string())
    }
}
