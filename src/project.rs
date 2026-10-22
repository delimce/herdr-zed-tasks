use crate::{herdr, Result};
use serde_json::Value;
use std::env;
use std::path::{Path, PathBuf};

pub fn active_root(workspace: &str, pane: Option<&str>) -> Result<PathBuf> {
    if let Ok(reply) = herdr::call(&["worktree".into(), "list".into()]) {
        if let Some(root) = root_from_worktrees(&reply["result"], workspace) {
            return Ok(root);
        }
    }

    if let Some(pane) = pane {
        if let Ok(reply) = herdr::call(&["pane".into(), "get".into(), pane.into()]) {
            let pane = &reply["result"]["pane"];
            for key in ["foreground_cwd", "cwd"] {
                if let Some(cwd) = pane[key].as_str() {
                    if let Some(root) = find_task_root(Path::new(cwd)) {
                        return Ok(root);
                    }
                }
            }
        }
    }

    let reply = herdr::call(&[
        "pane".into(),
        "list".into(),
        "--workspace".into(),
        workspace.into(),
    ])?;
    let mut roots = Vec::new();
    if let Some(panes) = reply["result"]["panes"].as_array() {
        for pane in panes {
            for key in ["foreground_cwd", "cwd"] {
                if let Some(cwd) = pane[key].as_str() {
                    if let Some(root) = find_task_root(Path::new(cwd)) {
                        if !roots.contains(&root) {
                            roots.push(root);
                        }
                    }
                }
            }
        }
    }
    match roots.len() {
        1 => Ok(roots.remove(0)),
        0 => Err("No .zed/tasks.json found for the active Herdr workspace".into()),
        _ => Err("Several projects with .zed/tasks.json share this workspace; focus a pane in the intended project".into()),
    }
}

pub fn root_from_worktrees(result: &Value, workspace: &str) -> Option<PathBuf> {
    if result["source"]["source_workspace_id"].as_str() == Some(workspace) {
        if let Some(path) = result["source"]["repo_root"].as_str() {
            let root = PathBuf::from(path);
            if root.join(".zed/tasks.json").is_file() {
                return Some(root);
            }
        }
    }
    result["worktrees"].as_array()?.iter().find_map(|worktree| {
        if worktree["open_workspace_id"].as_str() == Some(workspace) {
            let root = PathBuf::from(worktree["path"].as_str()?);
            root.join(".zed/tasks.json").is_file().then_some(root)
        } else {
            None
        }
    })
}

fn find_task_root(from: &Path) -> Option<PathBuf> {
    from.ancestors()
        .find(|dir| dir.join(".zed/tasks.json").is_file())
        .map(Path::to_path_buf)
}

pub fn source_context() -> Result<(String, Option<String>)> {
    let workspace = env::var("HZT_SOURCE_WORKSPACE")
        .or_else(|_| env::var("HERDR_WORKSPACE_ID"))
        .map_err(|_| "Missing source workspace")?;
    let pane = env::var("HZT_SOURCE_PANE")
        .or_else(|_| env::var("HERDR_PANE_ID"))
        .ok();
    Ok((workspace, pane))
}
