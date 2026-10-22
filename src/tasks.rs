use crate::Result;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub shell: Option<Value>,
}

pub struct Catalog {
    pub root: PathBuf,
    pub tasks: Vec<Task>,
    pub fingerprint: u64,
}

pub fn load(root: &Path) -> Result<Catalog> {
    let root = root.canonicalize()?;
    let source = fs::read(root.join(".zed/tasks.json"))?;
    let text = std::str::from_utf8(&source)?;
    let tasks: Vec<Task> = json5::from_str(text)?;
    if tasks.is_empty() {
        return Err(".zed/tasks.json contains no tasks".into());
    }
    for task in &tasks {
        if task.label.trim().is_empty() || task.command.trim().is_empty() {
            return Err("Each task needs a non-empty label and command".into());
        }
    }
    Ok(Catalog {
        root,
        tasks,
        fingerprint: fingerprint(&source),
    })
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut value = 0xcbf29ce484222325_u64;
    for byte in bytes {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x100000001b3);
    }
    value
}

pub fn variables(task: &Task) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for value in std::iter::once(&task.label)
        .chain(std::iter::once(&task.command))
        .chain(task.args.iter())
        .chain(task.cwd.iter())
        .chain(task.env.values())
    {
        scan_vars(value, &mut result);
    }
    result
}

fn scan_vars(value: &str, output: &mut BTreeSet<String>) {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        i += 1;
        let braced = bytes.get(i) == Some(&b'{');
        if braced {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        if i > start && value[start..i].starts_with("ZED_") {
            output.insert(value[start..i].to_owned());
        }
        if braced {
            while i < bytes.len() && bytes[i] != b'}' {
                i += 1;
            }
        }
    }
}

pub fn file_context_required(task: &Task) -> bool {
    variables(task).iter().any(|var| {
        matches!(
            var.as_str(),
            "ZED_FILE"
                | "ZED_FILENAME"
                | "ZED_DIRNAME"
                | "ZED_RELATIVE_FILE"
                | "ZED_RELATIVE_DIR"
                | "ZED_STEM"
        )
    })
}

pub fn unsupported_variables(task: &Task) -> Vec<String> {
    variables(task)
        .into_iter()
        .filter(|var| {
            !matches!(
                var.as_str(),
                "ZED_WORKTREE_ROOT"
                    | "ZED_MAIN_GIT_WORKTREE"
                    | "ZED_FILE"
                    | "ZED_FILENAME"
                    | "ZED_DIRNAME"
                    | "ZED_RELATIVE_FILE"
                    | "ZED_RELATIVE_DIR"
                    | "ZED_STEM"
            )
        })
        .collect()
}

pub fn search<'a>(tasks: &'a [Task], query: &str) -> Vec<(usize, &'a Task)> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| {
            let label = task.label.to_lowercase();
            words.iter().all(|word| label.contains(word))
        })
        .collect()
}
