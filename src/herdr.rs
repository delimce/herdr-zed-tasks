use crate::Result;
use serde_json::Value;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::Duration;

pub const PLUGIN_ID: &str = "local.zed-tasks";

pub fn call(args: &[String]) -> Result<Value> {
    let binary = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let output = Command::new(binary).args(args).output()?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Herdr command failed: {}", detail.trim()).into());
    }
    let response: Value = serde_json::from_slice(&output.stdout)?;
    if let Some(error) = response.get("error") {
        return Err(format!("Herdr error: {error}").into());
    }
    Ok(response)
}

pub fn open_picker() -> Result<()> {
    let workspace =
        env::var("HERDR_WORKSPACE_ID").map_err(|_| "Run this action from a Herdr workspace")?;
    let pane =
        env::var("HERDR_PANE_ID").map_err(|_| "Focus a terminal pane before opening Zed tasks")?;
    let args = vec![
        "plugin".into(),
        "pane".into(),
        "open".into(),
        "--plugin".into(),
        PLUGIN_ID.into(),
        "--entrypoint".into(),
        "picker".into(),
        "--placement".into(),
        "popup".into(),
        "--env".into(),
        format!("HZT_SOURCE_WORKSPACE={workspace}"),
        "--env".into(),
        format!("HZT_SOURCE_PANE={pane}"),
    ];
    call(&args)?;
    Ok(())
}

pub fn run_in_pane(
    workspace: &str,
    pane: &str,
    root: &std::path::Path,
    index: usize,
    fingerprint: u64,
    file: Option<&std::path::Path>,
) -> Result<()> {
    let pane_info = call(&["pane".into(), "get".into(), pane.into()])?;
    if pane_info["result"]["pane"]["workspace_id"].as_str() != Some(workspace) {
        return Err("The original terminal pane is no longer in this workspace".into());
    }
    let process_info = call(&[
        "pane".into(),
        "process-info".into(),
        "--pane".into(),
        pane.into(),
    ])?;
    let process_info = &process_info["result"]["process_info"];
    let shell_pid = process_info["shell_pid"].as_u64();
    let foreground_group = process_info["foreground_process_group_id"].as_u64();
    let foreground = process_info["foreground_processes"].as_array();
    let shell_ready = match (shell_pid, foreground_group, foreground) {
        (Some(shell), Some(group), Some(processes)) if shell == group => {
            processes.is_empty()
                || (processes.len() == 1 && processes[0]["pid"].as_u64() == Some(shell))
        }
        _ => false,
    };
    if !shell_ready {
        return Err("The original pane must be at its shell prompt to run a task here".into());
    }

    let binary = env::current_exe()?.canonicalize()?;
    let mut command = format!(
        "HZT_ROOT={} HZT_TASK_INDEX={index} HZT_TASK_FINGERPRINT={fingerprint}",
        shell_quote(&root.display().to_string())
    );
    if let Some(file) = file {
        command.push_str(&format!(
            " HZT_ZED_FILE={}",
            shell_quote(&file.display().to_string())
        ));
    }
    command.push_str(&format!(
        " {} run",
        shell_quote(&binary.display().to_string())
    ));
    call(&["pane".into(), "run".into(), pane.into(), command])?;
    Ok(())
}

pub fn close_picker_popup() -> Result<()> {
    let path = env::var("HERDR_SOCKET_PATH")
        .map_err(|_| "Herdr did not provide the socket for closing the task picker")?;
    let mut socket = UnixStream::connect(path)?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    socket.write_all(b"{\"id\":\"zed-tasks-close\",\"method\":\"popup.close\",\"params\":{}}\n")?;
    socket.flush()?;

    let mut response = String::new();
    BufReader::new(socket).read_line(&mut response)?;
    if response.is_empty() {
        return Err("Herdr closed the socket without confirming popup.close".into());
    }
    let response: Value = serde_json::from_str(&response)?;
    if let Some(error) = response.get("error") {
        return Err(format!("Herdr rejected popup.close: {error}").into());
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
