use self::reaper_web_client::rea_request::{Playstate, TransportResponse};

use super::*;
use crate::directory_shenanigans::project_directory;
pub mod common_types {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, strum::FromRepr, strum::Display)]
    pub enum ReaperBool {
        False = 0,
        True = 1,
    }
}

use futures::TryFutureExt;
use itertools::Itertools;
use reqwest::Url;
use std::{future::ready, sync::Arc};
use tui::{
    layout::Rect,
    style::{Color, Style},
    text::{Span, Spans, Text},
    widgets::{Paragraph, Wrap},
};

#[derive(Debug, Clone)]
pub struct ReaperInstance {
    process: Arc<RwLock<ProcessWatcher>>,
    state: Arc<RwLock<Result<reaper_web_client::rea_request::TransportResponse>>>,
    _web_client: Arc<reaper_web_client::ReaperWebClient>,
    _state_watcher: Arc<AbortOnDrop<()>>,
}

pub mod reaper_web_client;

impl ReaperInstance {
    #[instrument(ret, err)]
    pub async fn new(
        sessions_directory: SessionsDirectory,
        project_name: ProjectName,
        template: PathBuf,
        notify: ProcessEventBus,
        web_client_base_address: reqwest::Url,
    ) -> Result<Self> {
        let process_path = "reaper";
        let project_directory = project_directory(sessions_directory, &project_name)?;
        let project_file_path = project_directory
            .as_ref()
            .join(format!("{project_name}.rpp"));

        let (already_exists, command) = {
            let mut base = bounded_command(process_path);
            let base_command = base.arg("-nosplash").env("PIPEWIRE_LATENCY", "128/48000");
            let already_exists = project_file_path.exists();
            match already_exists {
                true => base_command.arg(project_file_path),
                false => base_command
                    .arg("-new")
                    .arg("-saveas")
                    .arg(project_file_path)
                    .arg("-template")
                    .arg(&template),
            }
            .spawn()
            .wrap_err("spawning process instance")
            .map(|command| (already_exists, command))?
        };

        ready(command)
            .then(|child| child.gracefully_shutdown_on_drop())
            .map_ok(|child| ProcessWatcher::new(process_path.to_owned(), child, notify.clone()))
            .map_ok(RwLock::new)
            .map_ok(Arc::new)
            .and_then(|child| {
                to_owned![notify];
                reaper_web_client::ReaperWebClient::new(web_client_base_address).and_then(
                    |web_client| async move {
                        let state = Arc::new(RwLock::new(Err(eyre!("Not started"))));
                        let state_watcher = {
                            to_owned![web_client, state, notify];
                            tokio::task::spawn(async move {
                                let mut tick = crate::process::app_interval(
                                    tokio::time::Duration::from_secs(1),
                                );
                                loop {
                                    tick.tick().await;
                                    *state.write() = web_client
                                        .clone()
                                        .run_single(reaper_web_client::rea_request::Transport)
                                        .await;
                                    if let Err(message) = notify.send(ProcessEvent::NewInput) {
                                        tracing::warn!(?message, "web client new data");
                                    }
                                }
                            })
                        }
                        .abort_on_drop();

                        if !already_exists {
                            web_client.clone().start_reaper_recording().await?;
                        }

                        Ok(Self {
                            process: child,
                            _web_client: web_client,
                            state,
                            _state_watcher: Arc::new(state_watcher),
                        })
                    },
                )
            })
            .await
    }
}

impl RenderToTerm for TransportResponse {
    fn render_to_term<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        let color = match self.playstate {
            Playstate::Stopped => Color::Gray,
            Playstate::Playing => Color::Green,
            Playstate::Paused => Color::Gray,
            Playstate::Recording => Color::LightRed,
            Playstate::RecordPaused => Color::DarkGray,
        };
        let lines = format!("{self:#?}",)
            .lines()
            .map(ToOwned::to_owned)
            .map(|line| Spans::from(Span::styled(line, Style::default().fg(color))))
            .collect_vec();
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().borders(Borders::ALL).title("Reaper state")),
            rect,
        );
        Ok(())
    }
}

impl<T: RenderToTerm> RenderToTerm for Option<T> {
    fn render_to_term<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        match self {
            Some(v) => v.render_to_term(f, rect),
            None => {
                f.render_widget(
                    Paragraph::new(Text::from(Spans::from(vec![Span::styled(
                        format!(" -- inactive ({}) --", std::any::type_name::<T>()),
                        Style::default().fg(Color::Red),
                    )])))
                    .wrap(Wrap { trim: false }),
                    rect,
                );
                Ok(())
            }
        }
    }
}

impl<T: RenderToTerm> RenderToTerm for Result<T, eyre::Report> {
    fn render_to_term<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        match self {
            Ok(v) => v.render_to_term(f, rect),
            Err(m) => {
                f.render_widget(
                    Paragraph::new(Text::from(Spans::from(vec![Span::styled(
                        format!("{m:?}"),
                        Style::default().fg(Color::Red),
                    )])))
                    .wrap(Wrap { trim: false }),
                    rect,
                );
                Ok(())
            }
        }
    }
}

impl RenderToTerm for ReaperInstance {
    fn render_to_term<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        let [state, logs]: [Rect; 2] = layout!(Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Ratio(1, 3), Constraint::Ratio(2, 3)])
            .split(rect));
        self.state.write().render_to_term(f, state)?;
        self.process.write().render_to_term(f, logs)?;

        Ok(())
    }
}

impl Drop for ReaperInstance {
    fn drop(&mut self) {
        use reaper_web_client::rea_request::ActionId;
        let web_client = self._web_client.clone();
        let process = self.process.clone();
        tokio::task::spawn(async move {
            let _process = process;
            for action in [ActionId::TransportStop, ActionId::SaveProject] {
                tracing::info!(?action, "cleaning up");
                if let Err(message) = web_client.clone().run_single(action).await {
                    tracing::error!(?message);
                }
            }
        });
    }
}
