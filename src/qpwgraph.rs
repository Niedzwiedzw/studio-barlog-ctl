use std::io::Write;

use super::*;

#[derive(Debug, Clone)]
pub struct QpwgraphInstance {
    process: Arc<RwLock<ProcessWatcher>>,
}

fn home_dir() -> Result<PathBuf> {
    directories::UserDirs::new()
        .ok_or_else(|| eyre!("no user dirs"))
        .map(|user| user.home_dir().to_owned())
}

/// qpwgraph has a bug so the file must be persistent...
fn temp_path() -> Result<PathBuf> {
    home_dir().map(|parent| parent.join("qpwgraph-reaper-generated-session.qpwgraph"))
}

pub trait ResultZipExt<T, E> {
    fn zip<U>(self, other: Result<U, E>) -> Result<(T, U), E>;
}

impl<T, E> ResultZipExt<T, E> for Result<T, E> {
    fn zip<U>(self, other: Result<U, E>) -> Result<(T, U), E> {
        match (self, other) {
            (Ok(one), Ok(other)) => Ok((one, other)),
            (Ok(_), Err(message)) => Err(message),
            (Err(message), Ok(_)) => Err(message),
            (Err(message), Err(_)) => Err(message),
        }
    }
}

impl QpwgraphInstance {
    const CONFIG: &str = include_str!("../reaper-session.qpwgraph");
    #[instrument(ret, err)]
    pub fn new(notify: ProcessEventBus) -> Result<Self> {
        let process_name = "qpwgraph".to_owned();
        home_dir()
            .zip(temp_path())
            .and_then(|(home_dir, temp_path)| {
                std::fs::write(&temp_path, Self::CONFIG.as_bytes())
                    .wrap_err("writing config")
                    .map(|_| temp_path)
                    .and_then(|temp_path| {
                        bounded_command(&process_name)
                            .current_dir(home_dir)
                            .arg(temp_path)
                            .spawn()
                            .wrap_err("spawning qpwgraph instance")
                            .map(|child| ProcessWatcher::new(process_name, child, notify))
                            .map(RwLock::new)
                            .map(Arc::new)
                            .map(|process| Self { process })
                    })
                    .wrap_err("spawning qpwgraph instance")
            })
    }
}

impl RenderToTerm for QpwgraphInstance {
    fn render_to_term<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        self.process.write().render_to_term(f, rect)?;

        Ok(())
    }
}
