use std::{future::ready, str::FromStr};

use tokio::process::Child;
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
    video_file_path: PathBuf,
    _ffplay: Arc<RwLock<ProcessWatcher>>,
    _file_size_updater: AbortOnDrop<()>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

pub fn ffplay_preview(video_device: VideoDevice) -> Result<Child> {
    bounded_command(FFPLAY_PATH)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .args(["-f", arg!("v4l2")])
        .args([arg!(video_device)])
        .spawn()
        .wrap_err("spawning ffmpeg instance")
}

impl FfmpegInstance {
    #[instrument(ret, err)]
    pub async fn new(
        video_device: VideoDevice,
        project_name: ProjectName,
        notify: ProcessEventBus,
    ) -> Result<Self> {
        let process_path = "ffmpeg".to_owned();
        let ffplay_path = "ffplay".to_owned();
        const RATE: usize = 25;
        const VIDEO_SIZE: &str = "1920x1080";

        ready(ffplay_preview(video_device.clone()))
            .and_then(|child| child.gracefully_shutdown_on_drop())
            .map_ok(|child| ProcessWatcher::new(ffplay_path, child, notify.clone()))
            .map_ok(RwLock::new)
            .map_ok(Arc::new)
            .and_then(|ffplay| {
                ready(video_file_path(&project_name)).and_then(|video_file_path| {
                    ready(
                        bounded_command(&process_path)
                            .args(["-thread_queue_size", arg!(512)])
                            .args(["-r", arg!(RATE)])
                            .args(["-f", arg!("v4l2")])
                            .args(["-video_size", arg!(VIDEO_SIZE)])
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
                                _ffplay: ffplay,
                                _file_size_updater: file_size_updater,
                            }
                        }
                    })
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
