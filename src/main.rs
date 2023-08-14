#![feature(async_fn_in_trait)]
use self::{qpwgraph::*, reaper::*, video_capture::*};
use clap::{Args, Parser, Subcommand};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use dioxus::prelude::*;
use directory_shenanigans::ExistingDirectory;
use eyre::{bail, eyre, Result, WrapErr};
use futures::{FutureExt, StreamExt, TryFutureExt};
use itertools::Itertools;
use parking_lot::RwLock;
use process::*;
use rendering::*;
use std::io;
use std::{
    future::ready,
    io::{Stdout, Write},
    path::PathBuf,
    process::Stdio,
    sync::Arc,
};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tracing::{debug, info, instrument, trace};
use tracing_subscriber::{fmt::Layer, prelude::*, EnvFilter};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders},
    Frame, Terminal,
};
use utils::*;

pub mod directory_shenanigans;
pub mod gst_viewer_dumper;
mod process;
pub mod qpwgraph;
pub mod reaper;
pub mod rendering;
pub mod space_available_watcher;
mod state;
pub mod utils;
pub mod video_capture;

#[derive(Debug, Clone, Copy)]
pub enum ProcessEvent {
    NewInput,
    ProcessExtied,
}

type ProcessEventBus = tokio::sync::mpsc::UnboundedSender<ProcessEvent>;

#[derive(Debug, Clone, derive_more::Display, derive_more::FromStr, derive_more::AsRef)]
pub struct ProjectName(String);

pub type ProjectTime = chrono::DateTime<chrono::Local>;

pub fn now() -> ProjectTime {
    chrono::Local::now()
}

fn bounded_command(path: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(path);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd
}

pub enum TracingKind {
    FileBased,
    TerminalBased,
}

fn setup_tracing(kind: TracingKind) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    EnvFilter::try_from_default_env()
        .ok()
        .and_then(|env_filter| match kind {
            TracingKind::FileBased => {
                let file_appender =
                    tracing_appender::rolling::daily(".", format!("{}.txt", clap::crate_name!()));
                let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
                tracing_subscriber::registry()
                    .with(
                        Layer::new()
                            .with_writer(non_blocking)
                            .with_filter(env_filter),
                    )
                    .init();
                Some(guard)
            }
            TracingKind::TerminalBased => {
                tracing_subscriber::fmt().init();
                None
            }
        })
}

#[derive(Debug, Clone, derive_more::FromStr, derive_more::AsRef)]
pub struct SessionsDirectory(ExistingDirectory);

#[derive(Args)]
pub struct MainConfig {
    /// specify base directory for all sessions
    #[arg(long, default_value = "/mnt/md0/manual-backup/reaper-sessions")]
    sessions_directory: SessionsDirectory,
    /// Project name to create
    #[arg(long)]
    project_name: ProjectName,
    /// Template to be used
    #[arg(long)]
    template: PathBuf,
    #[arg(long)]
    reaper_web_base_url: reqwest::Url,
    #[arg(long, value_parser = VideoDevice::new_checked)]
    video_device: VideoDevice,
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(next_line_help = true)]
struct Cli {
    // /// Sets a custom config file
    // #[arg(short, long, value_name = "FILE")]
    // config: Option<PathBuf>,

    // /// Turn debugging information on
    // #[arg(short, long, action = clap::ArgAction::Count)]
    // debug: u8,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    StartRecording(MainConfig),
    ShowVideos,
    QpwgraphOnly,
    GstViewerDumper(gst_viewer_dumper::Args),
}

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
    MainConfig {
        project_name,
        template,
        reaper_web_base_url,
        video_device,
        sessions_directory,
    }: MainConfig,
) -> Result<()> {
    let loopback_device = get_or_create_loopback_device_for(video_device).await?;
    state::StudioState::new(
        sessions_directory,
        project_name,
        template,
        reaper_web_base_url,
        loopback_device,
    )
    .and_then(|state| {
        enable_terminal_backend().and_then(|mut terminal| async move {
            ready(run_app(&mut terminal, state).await)
                .then(|app_result| {
                    disable_terminal_backend(terminal)
                        .map(move |term_result| app_result.and(term_result))
                })
                .await
        })
    })
    .then(|res| tokio::time::sleep(tokio::time::Duration::from_millis(100)).map(|_| res))
    .await
}

pub const CLEANUP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

async fn wait_for_accept(text: String) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        inquire::Select::new(&text, vec!["OK"])
            .prompt()
            .wrap_err("prompting for confirmation")
            .map(|_| ())
    })
    .await
    .wrap_err("thread crashed")
    .and_then(|v| v)
}

#[instrument]
async fn present_video_device(
    video_device: VideoDevice,
) -> Result<(GracefullyShutdownChild, VideoDevice)> {
    info!("presenting video device");
    video_capture::ffplay_preview(video_device.clone())
        .and_then(|ffplay| ffplay.gracefully_shutdown_on_drop())
        .await
        .map(|child| (child, video_device))
}

async fn app_main() -> Result<()> {
    let cli = Cli::parse();
    let _guard = setup_tracing(match cli.command {
        Commands::StartRecording(_) => TracingKind::FileBased,
        _ => TracingKind::TerminalBased,
    });
    match cli.command {
        Commands::ShowVideos => {
            let devices = video_capture::list_devices().await?;
            let _children = ready(devices)
                .then(|videos| {
                    futures::future::join_all(videos.iter().cloned().map(
                        |detailed_video_device| async move {
                            tracing::info!(?detailed_video_device);
                            present_video_device(detailed_video_device.video_device.clone())
                                .await
                                .wrap_err_with(move || {
                                    format!("spawning ffplay for {detailed_video_device:?}")
                                })
                        },
                    ))
                    .map(|v| v.into_iter().filter_map(|v| v.ok()).collect_vec())
                    .then(move |ready| {
                        let (children, devices): (Vec<_>, Vec<_>) = ready.into_iter().unzip();
                        wait_for_accept(format!(
                            "available devices: {:?}",
                            devices.into_iter().collect_vec()
                        ))
                        .map_ok(move |_| children)
                    })
                })
                .await?;
            Ok(())
        }
        Commands::StartRecording(config) => {
            {
                let video_device = config.video_device.clone();
                let (_child, video_device) = present_video_device(video_device.clone()).await?;
                wait_for_accept(format!("config video device {video_device}")).await?;
            }
            info!("chosen device, starting app");

            run_app_with_ui(config).await
        }
        Commands::QpwgraphOnly => {
            let (tx, _) = tokio::sync::mpsc::unbounded_channel();
            let _instance = qpwgraph::QpwgraphInstance::new(tx).await;
            wait_for_accept(format!("press anything to stop qpwgraph")).await?;
            Ok(())
        }
        Commands::GstViewerDumper(args) => {
            let viewer = gst_viewer_dumper::GStreamerReaderDumper::new(args)?;
            viewer.wait_for_finish().await?;
            Ok(())
        }
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    color_eyre::install().ok();

    // create app and run it
    let res = app_main().await;
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

            #[allow(clippy::single_match)]
            match ev {
                AppEvent::Terminal(event) => match event {
                    Event::Key(key) => match key.code {
                        KeyCode::Char('q')
                            if key
                                .modifiers
                                .contains(KeyModifiers::ALT & KeyModifiers::SHIFT) =>
                        {
                            return Ok(())
                        }
                        _ => {}
                    },
                    Event::FocusGained => redraw(),
                    Event::Resize(_, _) => redraw(),
                    Event::FocusLost => {}
                    Event::Mouse(_) => {}
                    Event::Paste(_) => {}
                },
                AppEvent::StateUpdated => redraw(),
            }
        }
    }
    Ok(())
}
