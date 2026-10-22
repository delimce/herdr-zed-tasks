mod herdr;
mod picker;
mod project;
mod runner;
mod tasks;

use std::env;
use std::error::Error;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() {
    if let Err(error) = execute() {
        eprintln!("Zed Tasks: {error}");
        std::process::exit(1);
    }
}

fn execute() -> Result<()> {
    match env::args().nth(1).as_deref() {
        Some("open") => herdr::open_picker(),
        Some("picker") => picker::run(),
        Some("run") => runner::run(),
        Some("check") => {
            let root = env::args()
                .nth(2)
                .ok_or("usage: herdr-zed-tasks check PROJECT_ROOT")?;
            let catalog = tasks::load(std::path::Path::new(&root))?;
            println!("{} task(s) available", catalog.tasks.len());
            Ok(())
        }
        Some("check-active") => {
            let (workspace, pane) = project::source_context()?;
            let root = project::active_root(&workspace, pane.as_deref())?;
            let catalog = tasks::load(&root)?;
            println!("{} task(s) available", catalog.tasks.len());
            Ok(())
        }
        _ => Err("usage: herdr-zed-tasks open|picker|run|check PROJECT_ROOT|check-active".into()),
    }
}
