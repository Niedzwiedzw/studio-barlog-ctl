use std::future::ready;

use super::*;
use crate::directory_shenanigans::{home_dir, temp_home_path};

#[derive(Debug, Clone)]
pub struct QpwgraphInstance {
    process: Arc<RwLock<ProcessWatcher>>,
}

impl QpwgraphInstance {
    const CONFIG: &str = include_str!("../reaper-session.qpwgraph");
    #[instrument(ret, err)]
    pub async fn new(notify: ProcessEventBus) -> Result<Self> {
        let process_name = "qpwgraph".to_owned();
        ready(home_dir().zip(temp_home_path("qpwgraph-reaper-generated-session.qpwgraph")))
            .and_then(|(home_dir, temp_path)| {
                tokio::fs::write(temp_path.clone(), Self::CONFIG.as_bytes())
                    .map(|v| v.wrap_err("writing config"))
                    .map_ok(|_| temp_path)
                    .and_then(|temp_path| {
                        ready(
                            bounded_command(&process_name)
                                .current_dir(home_dir)
                                .arg(temp_path)
                                .spawn()
                                .wrap_err("spawning qpwgraph instance"),
                        )
                        .and_then(|child| child.gracefully_shutdown_on_drop())
                        .map_ok(|child| ProcessWatcher::new(process_name, child, notify))
                        .map_ok(RwLock::new)
                        .map_ok(Arc::new)
                        .map_ok(|process| Self { process })
                    })
                    .map(|res| res.wrap_err("spawning qpwgraph instance"))
            })
            .await
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
