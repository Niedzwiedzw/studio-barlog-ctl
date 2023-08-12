use std::{future::ready, str::FromStr};

use once_cell::sync::Lazy;
use tokio::process::{Child, Command};
use tui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Paragraph, Wrap},
};

use crate::directory_shenanigans::{project_directory, ExistingDirectoryExt};

use super::*;

/// uses ffmpeg
///
/// shamelessly copied from:
/// #!/bin/bash
///
/// FFMPEGBIN="/usr/bin/ffmpeg"
/// RATE="50"
/// SIZE="1920x1080"
/// OUTPUTDIR="${PWD}"
/// FILE="magewell-recording-$(date +%Y-%m-%d--%H-%M-%S)--4k-23.98p.mov"
/// FILENAME="$OUTPUTDIR/$FILE"
/// VIDEODEVICE="/dev/video1"
///
/// "$FFMPEGBIN" \
///       -thread_queue_size 512 \
///       -r "$RATE" \
///       -f v4l2 \
///       -video_size "$SIZE" \
///       -i "$VIDEODEVICE" \
///       -crf 0 \
///       -c:v libx264 \
///       -preset ultrafast \
///       -threads 8  \
///       "$FILENAME"
///
/// echo "$FILENAME"
#[derive(Debug)]
pub struct FfmpegInstance {
    process: Arc<RwLock<ProcessWatcher>>,
    pub video_file_path: PathBuf,
    _file_size_updater: AbortOnDrop<()>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, derive_more::AsRef)]
pub struct VideoDevice(PathBuf);

impl VideoDevice {
    pub fn all() -> Result<Vec<Self>> {
        std::fs::read_dir("/dev/")
            .wrap_err("reading /dev")
            .map(|dev| {
                dev.into_iter()
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.as_os_str()
                            .to_str()
                            .map(|name| name.contains("video"))
                            .unwrap_or_default()
                    })
                    .map(Self)
                    .sorted()
                    .collect()
            })
    }

    pub fn new(value: &str) -> Result<Self, String> {
        Self::all()
            .wrap_err("unable to read video devices")
            .and_then(|devices| {
                PathBuf::from_str(value)
                    .wrap_err("not a valid path")
                    .map(Self)
                    .and_then(|valid| {
                        devices
                            .contains(&valid)
                            .then_some(valid)
                            .ok_or_else(|| eyre!("value not in {devices:?}"))
                    })
            })
            .map_err(|e| format!("{e:?}"))
    }
}

impl std::fmt::Display for VideoDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.display().fmt(f)
    }
}

fn video_file_path(project_name: &ProjectName) -> Result<PathBuf> {
    let now = crate::now().format("%Y-%m-%d--%H-%M-%S").to_string();
    project_directory(project_name)
        .and_then(|project_dir| {
            project_dir
                .as_ref()
                .join("video-recordings")
                .directory_exists()
        })
        .map(|project_video_dir| {
            project_video_dir
                .as_ref()
                .join(format!("{project_name}---{now}.mov"))
        })
}

const FFPLAY_PATH: &str = "ffplay";

macro_rules! arg {
    ($arg:expr) => {
        format!("{}", $arg).as_str()
    };
}

const VIDEO4LINUX2_FORMAT: &str = "v4l2";
const VIDEO_SIZE: &str = "1920x1080";
const RATE: usize = 25;

fn apply_video_read_args(command: &mut Command) -> &mut Command {
    command
        .args(["-f", arg!(VIDEO4LINUX2_FORMAT)])
        .args(["-video_size", arg!(VIDEO_SIZE)])
}

pub async fn ffplay_preview(video_device: VideoDevice) -> Result<Child> {
    try_enable_low_latency_for_magewell(video_device.clone()).await;
    let mut command = bounded_command(FFPLAY_PATH);
    #[cfg(not(debug_assertions))]
    {
        command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null());
    }
    #[cfg(debug_assertions)]
    {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    apply_video_read_args(&mut command)
        .args([arg!(video_device)])
        .spawn()
        .wrap_err_with(|| format!("spawning ffplay instance for {video_device:?}"))
}

pub const MWCAP_CONTROL_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| Default::default());

pub async fn try_enable_low_latency_for_magewell(video_device: VideoDevice) {
    if let Err(magewell_error) = enable_low_latency_for_magewell(video_device).await {
        tracing::error!(?magewell_error);
    }
}

#[instrument(ret, err, level = "info")]
pub async fn enable_low_latency_for_magewell(video_device: VideoDevice) -> Result<()> {
    let mutex = MWCAP_CONTROL_LOCK;
    let _guard = mutex.lock().await;
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    Command::new("mwcap-control")
        .arg("--video-output-lowlatency")
        .arg("on")
        .arg(video_device.as_ref())
        .output()
        .await
        .wrap_err("spawning command")
        .and_then(|output| {
            let out_text = format!(
                "stdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            output
                .status
                .success()
                .then_some(out_text.clone())
                .ok_or_else(|| eyre!("bad status: {}", output.status))
                .wrap_err_with(|| format!("output:\n\n{out_text}"))
        })
        .and_then(|output| {
            output
                .contains("successfully")
                .then_some(())
                .ok_or_else(|| eyre!("output text does not contain word 'successfully'"))
                .wrap_err_with(|| format!("checking output: {output}"))
        })
        .wrap_err_with(|| format!("enabling low latency mode for {video_device:?}"))
}

impl FfmpegInstance {
    #[instrument(ret, err)]
    pub async fn new(
        video_device: VideoDevice,
        project_name: ProjectName,
        notify: ProcessEventBus,
    ) -> Result<Self> {
        let process_path = "ffmpeg".to_owned();
        try_enable_low_latency_for_magewell(video_device.clone()).await;
        ready(video_file_path(&project_name))
            .and_then(|video_file_path| {
                let mut command = bounded_command(&process_path);
                ready(
                    apply_video_read_args(&mut command)
                        .args(["-thread_queue_size", arg!(512)])
                        .args(["-r", arg!(RATE)])
                        .args(["-i", arg!(video_device)])
                        .args(["-crf", arg!(0)])
                        .args(["-c:v", arg!("libx264")])
                        .args(["-preset", arg!("ultrafast")])
                        .args(["-threads", arg!(8)])
                        .args([&video_file_path])
                        .spawn()
                        .wrap_err("spawning ffmpeg instance"),
                )
                .and_then(|child| child.gracefully_shutdown_on_drop())
                .map_ok({
                    to_owned![notify];
                    move |child| ProcessWatcher::new(process_path, child, notify.clone())
                })
                .map_ok(RwLock::new)
                .map_ok(Arc::new)
                .map_ok({
                    to_owned![notify];
                    move |process| {
                        let file_size_updater = tokio::task::spawn(async move {
                            let mut interval =
                                tokio::time::interval(std::time::Duration::from_secs(1));

                            loop {
                                interval.tick().await;
                                notify.send(ProcessEvent::NewInput).ok();
                            }
                        })
                        .abort_on_drop();

                        Self {
                            process,
                            video_file_path,
                            _file_size_updater: file_size_updater,
                        }
                    }
                })
            })
            .await
    }

    pub fn file_size(&self) -> Result<String> {
        self.video_file_path
            .metadata()
            .wrap_err("reading file metadata")
            .map(|m| m.len())
            .map(|size| {
                byte_unit::Byte::from_bytes(size as _)
                    .get_appropriate_unit(true)
                    .to_string()
            })
    }
}

impl RenderToTerm for FfmpegInstance {
    fn render_to_term<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        let [file_size_block, process_watcher_block]: [Rect; 2] = layout!(Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Ratio(1, 9), Constraint::Ratio(8, 9),])
            .split(rect));
        let text_block = |text: String| {
            let block = Block::default().borders(Borders::ALL).title(Span::styled(
                format!("{}", self.video_file_path.display()),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
            Paragraph::new(text).block(block).wrap(Wrap { trim: false })
        };

        f.render_widget(
            text_block(
                self.file_size()
                    .unwrap_or_else(|e| format!("reading size: {e:?}")),
            ),
            file_size_block,
        );
        self.process
            .write()
            .render_to_term(f, process_watcher_block)?;
        Ok(())
    }
}
