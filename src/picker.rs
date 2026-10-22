use crate::{herdr, project, tasks, Result};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyModifiers,
};
use crossterm::style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};
use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

struct TerminalGuard;

impl TerminalGuard {
    fn new() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            Hide
        )?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            Show,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

enum Mode {
    Search,
    File { task_index: usize, input: String },
}

pub fn run() -> Result<()> {
    let workspace = env::var("HZT_SOURCE_WORKSPACE")?;
    let pane = env::var("HZT_SOURCE_PANE")?;
    let root = match project::active_root(&workspace, Some(&pane)) {
        Ok(root) => root,
        Err(error) => return show_error(&error.to_string()),
    };
    let catalog = match tasks::load(&root) {
        Ok(catalog) => catalog,
        Err(error) => return show_error(&error.to_string()),
    };
    let guard = TerminalGuard::new()?;
    let mut query = String::new();
    let mut selected = 0_usize;
    let mut mode = Mode::Search;
    let mut message = String::new();

    loop {
        let matches = tasks::search(&catalog.tasks, &query);
        if selected >= matches.len() {
            selected = matches.len().saturating_sub(1);
        }
        draw(&catalog, &query, &matches, selected, &mode, &message)?;
        match event::read()? {
            Event::Key(key) if key.kind == event::KeyEventKind::Press => {
                if key.code == KeyCode::Esc
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    if matches!(mode, Mode::File { .. }) {
                        mode = Mode::Search;
                        message.clear();
                        continue;
                    }
                    return Ok(());
                }
                match &mut mode {
                    Mode::Search => match key.code {
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            query.clear();
                            selected = 0;
                            message.clear();
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            query.push(c);
                            selected = 0;
                            message.clear();
                        }
                        KeyCode::Backspace => {
                            query.pop();
                            selected = 0;
                            message.clear();
                        }
                        KeyCode::Down => {
                            if !matches.is_empty() {
                                selected = (selected + 1).min(matches.len() - 1);
                            }
                        }
                        KeyCode::Up => {
                            selected = selected.saturating_sub(1);
                        }
                        KeyCode::Enter => {
                            let Some((index, task)) = matches.get(selected).copied() else {
                                continue;
                            };
                            let unsupported = tasks::unsupported_variables(task);
                            if !unsupported.is_empty() {
                                message =
                                    format!("Unavailable Zed context: {}", unsupported.join(", "));
                            } else if tasks::file_context_required(task) {
                                mode = Mode::File {
                                    task_index: index,
                                    input: String::new(),
                                };
                                message.clear();
                            } else {
                                match herdr::run_in_pane(
                                    &workspace,
                                    &pane,
                                    &catalog.root,
                                    index,
                                    catalog.fingerprint,
                                    None,
                                ) {
                                    Ok(()) => return finish_selection(guard),
                                    Err(error) => message = error.to_string(),
                                }
                            }
                        }
                        _ => {}
                    },
                    Mode::File { task_index, input } => match key.code {
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            input.clear();
                            message.clear();
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            input.push(c);
                            message.clear();
                        }
                        KeyCode::Backspace => {
                            input.pop();
                            message.clear();
                        }
                        KeyCode::Enter => match resolve_file(&catalog.root, input) {
                            Ok(file) => {
                                match herdr::run_in_pane(
                                    &workspace,
                                    &pane,
                                    &catalog.root,
                                    *task_index,
                                    catalog.fingerprint,
                                    Some(&file),
                                ) {
                                    Ok(()) => return finish_selection(guard),
                                    Err(error) => message = error.to_string(),
                                }
                            }
                            Err(error) => message = error.to_string(),
                        },
                        _ => {}
                    },
                }
            }
            Event::Paste(text) => match &mut mode {
                Mode::Search => {
                    query.push_str(&text);
                    selected = 0;
                    message.clear();
                }
                Mode::File { input, .. } => {
                    input.push_str(&text);
                    message.clear();
                }
            },
            _ => {}
        }
    }
}

fn finish_selection(guard: TerminalGuard) -> Result<()> {
    drop(guard);
    herdr::close_picker_popup()?;
    Ok(())
}

fn resolve_file(root: &Path, input: &str) -> Result<PathBuf> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter a file path, or press Escape".into());
    }
    let path = if input == "~" || input.starts_with("~/") {
        PathBuf::from(env::var("HOME")?).join(input.trim_start_matches('~').trim_start_matches('/'))
    } else {
        PathBuf::from(input)
    };
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    let path = path.canonicalize().map_err(|_| "File does not exist")?;
    if !path.is_file() || !path.starts_with(root) {
        return Err("Choose an existing file inside this project".into());
    }
    Ok(path)
}

fn draw(
    catalog: &tasks::Catalog,
    query: &str,
    matches: &[(usize, &tasks::Task)],
    selected: usize,
    mode: &Mode,
    message: &str,
) -> Result<()> {
    let (width, height) = terminal::size()?;
    let mut out = io::stdout();
    queue!(out, MoveTo(0, 0), Clear(ClearType::All))?;
    let title = format!(
        "Zed tasks v{} · {}",
        env!("CARGO_PKG_VERSION"),
        catalog.root.display()
    );
    line(&mut out, 0, width, &title)?;
    match mode {
        Mode::Search => {
            line(&mut out, 2, width, &format!("Search label: {query}"))?;
            line(
                &mut out,
                3,
                width,
                &format!("{} matching task(s)", matches.len()),
            )?;
            let visible = height.saturating_sub(7) as usize;
            let start = if selected >= visible && visible > 0 {
                selected + 1 - visible
            } else {
                0
            };
            for (row, (_, task)) in matches.iter().skip(start).take(visible).enumerate() {
                let is_selected = start + row == selected;
                let marker = if is_selected { "> " } else { "  " };
                task_line(
                    &mut out,
                    4 + row as u16,
                    width,
                    &format!("{marker}{}", task.label),
                    is_selected,
                )?;
            }
            if height > 1 {
                line(
                    &mut out,
                    height - 2,
                    width,
                    "Type to filter · ↑/↓ select · Enter run · Esc close",
                )?;
            }
        }
        Mode::File { task_index, input } => {
            line(
                &mut out,
                2,
                width,
                &format!("Task: {}", catalog.tasks[*task_index].label),
            )?;
            line(
                &mut out,
                4,
                width,
                "This task uses $ZED_FILE. Enter a project file path:",
            )?;
            line(&mut out, 5, width, input)?;
            if height > 1 {
                line(&mut out, height - 2, width, "Enter run · Esc back")?;
            }
        }
    }
    if height > 0 {
        line(&mut out, height - 1, width, message)?;
    }
    out.flush()?;
    Ok(())
}

fn line(out: &mut impl Write, row: u16, width: u16, text: &str) -> Result<()> {
    task_line(out, row, width, text, false)
}

fn task_line(out: &mut impl Write, row: u16, width: u16, text: &str, selected: bool) -> Result<()> {
    let visible: String = text
        .chars()
        .filter(|c| !c.is_control())
        .take(width as usize)
        .collect();
    if selected {
        queue!(
            out,
            MoveTo(0, row),
            SetForegroundColor(Color::Black),
            SetBackgroundColor(Color::Yellow),
            Print(visible),
            ResetColor
        )?;
    } else {
        queue!(out, MoveTo(0, row), Print(visible))?;
    }
    Ok(())
}

fn show_error(message: &str) -> Result<()> {
    let _guard = TerminalGuard::new()?;
    let (width, _) = terminal::size()?;
    let mut out = io::stdout();
    queue!(out, Clear(ClearType::All))?;
    line(&mut out, 0, width, "Zed tasks")?;
    line(&mut out, 2, width, message)?;
    line(&mut out, 4, width, "Press any key to close")?;
    out.flush()?;
    loop {
        if let Event::Key(_) = event::read()? {
            return Ok(());
        }
    }
}
