use self::components::*;
use self::qpwgraph::*;
use self::reaper::*;
use self::video_capture::*;
use clap::{Parser, Subcommand};
use dioxus::prelude::*;
use eyre::{bail, eyre, Result, WrapErr};
use futures::FutureExt;
use futures::StreamExt;
use futures::TryFutureExt;
use itertools::Itertools;
use parking_lot::RwLock;
use std::io::Stdout;
use std::io::Write;
use std::path::PathBuf;
use tracing::trace;

use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::BufReader;
use tracing::debug;
use tracing::{info, instrument};
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;
pub mod qpwgraph;
pub mod rendering;
use rendering::*;

#[derive(Debug, Clone, Copy)]
pub enum ProcessEvent {
    NewInput,
    ProcessExtied,
}

type ProcessEventBus = tokio::sync::mpsc::UnboundedSender<ProcessEvent>;

pub mod directory_shenanigans;
mod process;
pub mod utils;
use process::*;
use utils::*;
pub mod reaper;
pub mod video_capture;
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

#[derive(Debug, Clone, derive_more::Display, derive_more::FromStr, derive_more::AsRef)]
pub struct ProjectName(String);

mod state;
use state::*;

pub type ProjectTime = chrono::DateTime<chrono::Local>;

pub fn now() -> ProjectTime {
    chrono::Local::now()
}

fn bounded_command(path: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(path);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
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

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(next_line_help = true)]
struct Cli {
    /// Project name to create
    #[arg(long)]
    project_name: ProjectName,
    /// Template to be used
    #[arg(long)]
    template: PathBuf,
    #[arg(long)]
    reaper_web_base_url: reqwest::Url,
    #[arg(long, value_parser = VideoDevice::new)]
    video_device: VideoDevice,
    // /// Sets a custom config file
    // #[arg(short, long, value_name = "FILE")]
    // config: Option<PathBuf>,

    // /// Turn debugging information on
    // #[arg(short, long, action = clap::ArgAction::Count)]
    // debug: u8,

    // #[command(subcommand)]
    // command: Option<Commands>,
}

// #[derive(Subcommand)]
// enum Commands {
//     /// does testing things
//     Test {
//         /// lists test values
//         #[arg(short, long)]
//         list: bool,
//     },
// }

type AppTerminal = Terminal<CrosstermBackend<Stdout>>;

#[instrument(err, level = "warn")]
async fn enable_terminal_backend() -> Result<AppTerminal> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;

    Ok(terminal)
}

#[instrument(skip(terminal), ret, err, level = "warn")]
async fn disable_terminal_backend(mut terminal: AppTerminal) -> Result<()> {
    tracing::warn!("closing, cleaning up the terminal backend");
    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    std::io::stdout()
        .lock()
        .flush()
        .wrap_err("flushing stdout")?;
    // this resets the temrinal just in case
    print!("\x1b[0m");
    Ok(())
}

async fn run_app_with_ui(
    Cli {
        project_name,
        template,
        reaper_web_base_url,
        video_device,
    }: Cli,
) -> Result<()> {
    use std::future::ready;
    state::StudioState::new(project_name, template, reaper_web_base_url, video_device)
        .and_then(|state| {
            enable_terminal_backend().and_then(|mut terminal| async move {
                ready(run_app(&mut terminal, state).await)
                    .then(|app_result| {
                        disable_terminal_backend(terminal)
                            .map(move |term_result| app_result.and_then(move |_| term_result))
                    })
                    .await
            })
        })
        .then(|res| tokio::time::sleep(tokio::time::Duration::from_millis(100)).map(|_| res))
        .await
}

pub const CLEANUP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let _guard = setup_tracing();

    let cli = Cli::parse();
    // create app and run it
    let res = run_app_with_ui(cli).await;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    println!(
        "waiting for {}s - processess are cleaning up\n\n\r\n",
        CLEANUP_DEADLINE.as_secs()
    );
    tokio::time::sleep(CLEANUP_DEADLINE).await;

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
            trace!(?ev, "new event");

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
