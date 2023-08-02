use super::*;
use crate::directory_shenanigans::{home_dir, temp_path};

#[derive(Debug, Clone)]
pub struct QpwgraphInstance {
    process: Arc<RwLock<ProcessWatcher>>,
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
                            .map(|child| {
                                ProcessWatcher::new(
                                    process_name,
                                    child.gracefully_shutdown_on_drop(),
                                    notify,
                                )
                            })
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
