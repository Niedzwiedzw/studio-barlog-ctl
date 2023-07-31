use std::sync::Arc;

use itertools::Itertools;
use tui::{
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{List, ListItem},
};

use crate::directory_shenanigans::ExistingDirectory;

use super::*;

#[derive(Debug, Clone)]
pub struct ReaperInstance {
    process: Arc<RwLock<ProcessWatcher>>,
}

fn sessions_directory() -> Result<ExistingDirectory> {
    crate::directory_shenanigans::home_dir()
        .map(|home| home.join("Music").join("reaper-sessions"))
        .and_then(ExistingDirectory::check)
}

impl ReaperInstance {
    #[instrument(ret, err)]
    pub fn new(
        project_name: ProjectName,
        template: PathBuf,
        notify: ProcessEventBus,
    ) -> Result<Self> {
        let path = "reaper".to_owned();
        sessions_directory().and_then(|sessions_directory| {
            bounded_command(&path)
                .arg("-new")
                .arg("-saveas")
                .arg(
                    sessions_directory
                        .as_ref()
                        .join(format!("{project_name}.rpp")),
                )
                .arg("-template")
                .arg(&template)
                .arg("-nosplash")
                .env("PIPEWIRE_LATENCY", "128/48000")
                .spawn()
                .wrap_err("spawning process instance")
                .map(|child| ProcessWatcher::new(path, child, notify))
                .map(RwLock::new)
                .map(Arc::new)
                .map(|process| Self { process })
        })
    }
}

impl RenderToTerm for ReaperInstance {
    fn render_to_term<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        self.process.write().render_to_term(f, rect)?;

        Ok(())
    }
}
