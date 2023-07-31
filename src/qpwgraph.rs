use std::io::Write;

use super::*;

#[derive(Debug, Clone)]
pub struct QpwgraphInstance {
    process: Arc<RwLock<ProcessWatcher>>,
}

/// qpwgraph has a bug so the file must be persistent...
fn temp_path() -> Result<PathBuf> {
    std::env::current_dir()
        .wrap_err("temporary directory unavailable")
        .map(|parent| parent.join("qpwgraph-reaper-generated-session.qpwgraph"))
}

impl QpwgraphInstance {
    const CONFIG: &str = include_str!("../reaper-session.qpwgraph");
    #[instrument(ret, err)]
    pub fn new(notify: ProcessEventBus) -> Result<Self> {
        let process_name = "qpwgraph".to_owned();
        temp_path()
            .and_then(|temp_path| {
                std::fs::write(&temp_path, Self::CONFIG.as_bytes())
                    .wrap_err("writing config")
                    .map(|_| temp_path)
            })
            .and_then(|temp_path| {
                bounded_command(&process_name)
                    .arg(temp_path)
                    .spawn()
                    .wrap_err("spawning qpwgraph instance")
                    .map(|child| ProcessWatcher::new(process_name, child, notify))
                    .map(RwLock::new)
                    .map(Arc::new)
                    .map(|process| Self { process })
            })
            .wrap_err("spawning qpwgraph instance")
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
