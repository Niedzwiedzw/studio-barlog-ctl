use tui::{
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Paragraph, Wrap},
};

use super::*;
#[derive(Debug)]
pub struct SpaceAvailableWatcher {
    directory: PathBuf,
    stdout: Arc<RwLock<String>>,
    _watcher: AbortOnDrop<()>,
}

impl SpaceAvailableWatcher {
    pub fn new(target_directory: PathBuf) -> Self {
        let stdout = Arc::new(RwLock::new(String::new()));
        let directory = target_directory.clone();
        let watcher = {
            to_owned![stdout];
            tokio::task::spawn(async move {
                let mut interval =
                    crate::process::app_interval(tokio::time::Duration::from_secs(1));
                loop {
                    interval.tick().await;
                    tokio::process::Command::new("df")
                        .arg("-P")
                        .arg("-h")
                        .arg(&target_directory)
                        .output()
                        .await
                        .wrap_err("process failed")
                        .and_then(|output| {
                            let status = output.status;
                            output
                                .status
                                .success()
                                .then_some(output)
                                .ok_or_else(move || eyre!("exited [{:?}]", status))
                        })
                        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
                        .map(|output| *stdout.write() = output)
                        .unwrap_or_else(|error| *stdout.write() = format!("{error:?}"));
                }
            })
            .abort_on_drop()
        };
        Self {
            directory,
            stdout,
            _watcher: watcher,
        }
    }
}

impl RenderToTerm for SpaceAvailableWatcher {
    fn render_to_term<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        let text_block = |text: &str| {
            let block = Block::default().borders(Borders::ALL).title(Span::styled(
                format!("available: [{}]", self.directory.display()),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
            Paragraph::new(text.to_owned())
                .block(block)
                .wrap(Wrap { trim: false })
        };
        f.render_widget(text_block(self.stdout.read().as_str()), rect);
        Ok(())
    }
}
