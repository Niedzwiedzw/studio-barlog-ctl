use self::components::*;
use self::qpwgraph::*;
use self::reaper::*;
use self::video_streamer::*;
use dioxus::prelude::*;
use eyre::{bail, eyre, Result, WrapErr};
use futures::AsyncBufRead;
use futures::FutureExt;
use futures::StreamExt;
use parking_lot::RwLock;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::BufReader;
use tracing::debug;
use tracing::span;
use tracing::Subscriber;
use tracing::{info, instrument};
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;
pub mod qpwgraph;
pub mod rendering;
use rendering::*;
type Notify = tokio::sync::mpsc::UnboundedSender<()>;
mod process;
use process::*;
pub mod reaper;
pub mod video_streamer {
    use super::*;
}
pub mod components {
    use super::*;
}
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{error::Error, io};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders},
    Frame, Terminal,
};

#[derive(Debug, Clone, derive_more::Display)]
pub struct ProjectName(String);

mod state;
use state::*;

pub type ProjectTime = chrono::DateTime<chrono::Local>;

pub fn now() -> ProjectTime {
    chrono::Local::now()
}

fn bounded_command(path: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(path);
    cmd.kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn setup_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    EnvFilter::try_from_default_env().ok().map(|env_filter| {
        let file_appender =
            tracing_appender::rolling::hourly("./", format!("{}.txt", clap::crate_name!()));
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

        tracing_subscriber::registry()
            .with(
                Layer::new()
                    .with_writer(non_blocking)
                    .with_filter(env_filter),
            )
            .init();
        guard
    })
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let _guard = setup_tracing();
    info!("starting");
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let state =
        state::StudioState::new(ProjectName("Demo".to_owned())).wrap_err("initializing project")?;

    // create app and run it
    let res = run_app(&mut terminal, state).await;

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

async fn next_event() -> Result<crossterm::event::Event> {
    tokio::task::spawn_blocking(|| event::read().wrap_err("reading next event"))
        .await
        .map_err(|e| eyre!("{e:?}"))
        .wrap_err("joining asynchronous task")
        .and_then(|v| v)
}

#[derive(Debug)]
pub enum AppEvent {
    Terminal(crossterm::event::Event),
    StateUpdated,
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut state: state::StudioState,
) -> Result<()> {
    let term_events = futures::stream::unfold((), |_| next_event().map(|v| Some((v, ()))))
        .map(|event| event.map(AppEvent::Terminal))
        .boxed();
    let wake_up = state
        .wake_up
        .take()
        .ok_or_else(|| eyre!("notifier not initialized - programming error"))?
        .map(|_| Result::<AppEvent, eyre::Report>::Ok(AppEvent::StateUpdated))
        .boxed();
    let mut app_events = futures::stream::select_all([term_events, wake_up]);

    let mut redraw = || {
        debug!("redrawing");
        terminal
            .draw(|f| {
                state.render_to_term(f, f.size()).ok();
            })
            .ok();
    };
    redraw();
    while let Some(ev) = app_events.next().await {
        if let Ok(ev) = ev {
            debug!(?ev, "new event");

            match ev {
                AppEvent::Terminal(event) => match event {
                    Event::Key(key) => match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        _ => {}
                    },
                    Event::FocusGained => redraw(),
                    Event::FocusLost => {}
                    Event::Mouse(_) => {}
                    Event::Paste(_) => {}
                    Event::Resize(_, _) => redraw(),
                },
                AppEvent::StateUpdated => redraw(),
            }
        }
    }
    Ok(())
}
