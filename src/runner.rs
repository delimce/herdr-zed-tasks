use crate::{tasks, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

pub fn run() -> Result<()> {
    let status = execute()?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

fn execute() -> Result<ExitStatus> {
    let root = PathBuf::from(env::var("HZT_ROOT")?);
    let index: usize = env::var("HZT_TASK_INDEX")?.parse()?;
    let expected: u64 = env::var("HZT_TASK_FINGERPRINT")?.parse()?;
    let catalog = tasks::load(&root)?;
    if catalog.fingerprint != expected {
        return Err(".zed/tasks.json changed after selection; open the picker again".into());
    }
    let task = catalog
        .tasks
        .get(index)
        .ok_or("Selected task no longer exists")?;
    let unsupported = tasks::unsupported_variables(task);
    if !unsupported.is_empty() {
        return Err(format!("Unavailable Zed context: {}", unsupported.join(", ")).into());
    }

    let mut values: HashMap<String, String> = env::vars().collect();
    values.insert(
        "ZED_WORKTREE_ROOT".into(),
        catalog.root.display().to_string(),
    );
    values.insert(
        "ZED_MAIN_GIT_WORKTREE".into(),
        main_git_worktree(&catalog.root).display().to_string(),
    );
    if let Ok(file) = env::var("HZT_ZED_FILE") {
        add_file_context(&mut values, &catalog.root, Path::new(&file))?;
    } else if tasks::file_context_required(task) {
        return Err("This task needs a file path; reopen it from the picker".into());
    }

    for (key, value) in &task.env {
        let expanded = expand(value, &values)?;
        values.insert(key.clone(), expanded);
    }
    let cwd = match &task.cwd {
        Some(cwd) => {
            let expanded = PathBuf::from(expand(cwd, &values)?);
            if expanded.is_absolute() {
                expanded
            } else {
                catalog.root.join(expanded)
            }
        }
        None => catalog.root.clone(),
    };
    if !cwd.is_dir() {
        return Err("Task working directory does not exist".into());
    }

    let args: Vec<String> = task
        .args
        .iter()
        .map(|arg| expand(arg, &values))
        .collect::<Result<_>>()?;
    let command_line = if args.is_empty() {
        task.command.clone()
    } else {
        format!(
            "{} {}",
            task.command,
            args.iter()
                .map(|arg| shell_quote(arg))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    let (shell, shell_args) = shell_command(task.shell.as_ref())?;
    let mut command = Command::new(shell);
    command
        .args(shell_args)
        .arg("-c")
        .arg(command_line)
        .current_dir(&cwd);
    command.envs(&values);
    for (key, _) in env::vars() {
        if key.starts_with("HERDR_PLUGIN_") || key.starts_with("HZT_") {
            command.env_remove(key);
        }
    }
    println!("Task: {}", task.label);
    println!("Directory: {}\n", cwd.display());
    let status = command.status()?;
    match status.code() {
        Some(code) => println!("\nTask finished with exit code {code}."),
        None => println!("\nTask interrupted."),
    }
    Ok(status)
}

fn shell_command(value: Option<&Value>) -> Result<(String, Vec<String>)> {
    match value {
        None | Some(Value::String(_)) if value.map_or(true, |v| v.as_str() == Some("system")) => {
            let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            Ok((shell, vec!["-l".into()]))
        }
        Some(Value::Object(map)) if map.contains_key("program") => {
            let program = map["program"]
                .as_str()
                .ok_or("Invalid task shell program")?;
            Ok((program.into(), Vec::new()))
        }
        Some(Value::Object(map)) if map.contains_key("with_arguments") => {
            let item = &map["with_arguments"];
            let program = item["program"]
                .as_str()
                .ok_or("Invalid task shell program")?;
            let args = item["args"]
                .as_array()
                .ok_or("Invalid task shell arguments")?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or("Invalid task shell argument")
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok((program.into(), args))
        }
        _ => Err("Unsupported task shell configuration".into()),
    }
}

fn add_file_context(values: &mut HashMap<String, String>, root: &Path, file: &Path) -> Result<()> {
    let file = file.canonicalize()?;
    if !file.is_file() || !file.starts_with(root) {
        return Err("Selected file must exist inside the project".into());
    }
    let relative = file.strip_prefix(root)?;
    let filename = file
        .file_name()
        .ok_or("Invalid file name")?
        .to_string_lossy()
        .into_owned();
    let stem = file
        .file_stem()
        .ok_or("Invalid file stem")?
        .to_string_lossy()
        .into_owned();
    values.insert("ZED_FILE".into(), file.display().to_string());
    values.insert("ZED_FILENAME".into(), filename);
    values.insert(
        "ZED_DIRNAME".into(),
        file.parent()
            .ok_or("Invalid file directory")?
            .display()
            .to_string(),
    );
    values.insert("ZED_RELATIVE_FILE".into(), relative.display().to_string());
    values.insert(
        "ZED_RELATIVE_DIR".into(),
        relative
            .parent()
            .unwrap_or(Path::new(""))
            .display()
            .to_string(),
    );
    values.insert("ZED_STEM".into(), stem);
    Ok(())
}

fn main_git_worktree(root: &Path) -> PathBuf {
    let output = Command::new("git")
        .args([
            "-C",
            &root.display().to_string(),
            "rev-parse",
            "--git-common-dir",
        ])
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            if let Ok(path) = std::str::from_utf8(&output.stdout) {
                let path = PathBuf::from(path.trim());
                let path = if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                };
                if let Ok(path) = path.canonicalize() {
                    if path.file_name().is_some_and(|name| name == ".git") {
                        if let Some(parent) = path.parent() {
                            return parent.to_path_buf();
                        }
                    }
                }
            }
        }
    }
    root.to_path_buf()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn expand(input: &str, values: &HashMap<String, String>) -> Result<String> {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' || (i > 0 && bytes[i - 1] == b'\\') {
            i += 1;
            continue;
        }
        output.push_str(&input[start..i]);
        let dollar = i;
        i += 1;
        let braced = bytes.get(i) == Some(&b'{');
        if braced {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        if i == name_start {
            output.push('$');
            start = dollar + 1;
            i = start;
            continue;
        }
        let name = &input[name_start..i];
        let mut default = None;
        if braced {
            if bytes.get(i) == Some(&b':') {
                i += 1;
                let default_start = i;
                while i < bytes.len() && bytes[i] != b'}' {
                    i += 1;
                }
                default = Some(&input[default_start..i]);
            }
            if bytes.get(i) != Some(&b'}') {
                return Err(format!("Unclosed task variable {name}").into());
            }
            i += 1;
        }
        let value = values
            .get(name)
            .map(String::as_str)
            .or(default)
            .ok_or_else(|| format!("Task variable {name} is unavailable"))?;
        output.push_str(value);
        start = i;
    }
    output.push_str(&input[start..]);
    Ok(output)
}
