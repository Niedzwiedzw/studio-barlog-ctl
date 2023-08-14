use super::*;
use crate::directory_shenanigans::{project_directory, ExistingDirectoryExt};
use once_cell::sync::Lazy;
use std::{collections::HashSet, future::ready, process::Output};
use tokio::process::{Child, Command};
use tui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Paragraph, Wrap},
};
pub mod gstreamer_process;

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
    preview_process: Arc<RwLock<Result<ProcessWatcher>>>,
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

    pub fn new(value: &str) -> Result<Self> {
        value.parse().wrap_err("invalid path").map(Self)
    }

    pub fn new_checked(value: &str) -> Result<Self> {
        Self::all()
            .wrap_err("unable to read video devices")
            .and_then(|devices| {
                Self::new(value).and_then(|valid| {
                    devices
                        .contains(&valid)
                        .then(|| valid.clone())
                        .ok_or_else(|| eyre!("device {valid:?} not in {devices:?}"))
                })
            })
    }
}

impl std::fmt::Display for VideoDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.display().fmt(f)
    }
}

pub fn video_file_path(
    sessions_directory: SessionsDirectory,
    project_name: &ProjectName,
) -> Result<PathBuf> {
    let now = crate::now().format("%Y-%m-%d--%H-%M-%S").to_string();
    project_directory(sessions_directory, project_name)
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

#[derive(Debug, Clone)]
pub struct SuccessOutput {
    pub stdout: String,
    pub stderr: String,
}

pub trait OutputExt {
    fn success_output(self) -> Result<SuccessOutput>;
}

impl OutputExt for Output {
    fn success_output(self) -> Result<SuccessOutput> {
        let output = SuccessOutput {
            stdout: String::from_utf8_lossy(&self.stdout).to_string(),
            stderr: String::from_utf8_lossy(&self.stderr).to_string(),
        };
        self.status
            .success()
            .then(|| output.clone())
            .ok_or_else(|| eyre!("bad status: {}", self.status))
            .wrap_err_with(|| {
                format!(
                    "output:\nstdout: {}\nstderr: {}\n",
                    output.stdout, output.stderr
                )
            })
    }
}

#[derive(Hash, Debug, PartialEq, Eq, Clone)]
pub struct DetailedVideoDevice {
    pub video_device: VideoDevice,
    pub details: String,
}

#[tracing::instrument(ret, err, level = "DEBUG")]
pub async fn list_devices() -> Result<Vec<DetailedVideoDevice>> {
    #[tracing::instrument(ret, err, level = "DEBUG")]
    fn parse_section(section: &str) -> Result<Vec<DetailedVideoDevice>> {
        section
            .split_once('\n')
            .into_iter()
            .flat_map(|(header, devices)| {
                tracing::debug!(%header, %devices);
                devices
                    .split('\n')
                    .filter_map(|v| {
                        let v = v.trim();
                        (!v.is_empty()).then_some(v)
                    })
                    .map(|device| {
                        VideoDevice::new(device).map(|video_device| DetailedVideoDevice {
                            video_device,
                            details: header.trim().to_string(),
                        })
                    })
            })
            .collect()
    }
    Command::new("v4l2-ctl")
        .arg("--list-devices")
        .output()
        .await
        .wrap_err("reading command output")
        .and_then(|output| output.success_output())
        .and_then(|SuccessOutput { stdout, stderr }| {
            tracing::debug!(%stdout, %stderr, "success output");
            stdout
                .split("\n\n")
                .map(parse_section)
                .collect::<Result<Vec<_>>>()
                .map(|v| v.into_iter().flatten().collect::<Vec<_>>())
        })
        .wrap_err("reading current video devices")
}

#[derive(Debug)]
pub struct LoopbackDevice {
    pub loopback_device: VideoDevice,
    pub for_device: VideoDevice,
}

fn video_nr_from_video(device: &VideoDevice) -> Result<i32> {
    device
        .0
        .display()
        .to_string()
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .wrap_err("extracting video device number")
}

fn loopback_label(device: &VideoDevice) -> Result<String> {
    video_nr_from_video(device)
        .map(|number| format!("loopback-device-for-{number}"))
        .wrap_err("generating loopback label")
}

/// sudo modprobe v4l2loopback video_nr=1 card_label=video-loopback-1 exclusive_caps=1
pub async fn get_or_create_loopback_device_for(
    for_video_device: VideoDevice,
) -> Result<LoopbackDevice> {
    // if let Err() =
    // Command::new("sudo")
    //     .stdin(Stdio::inherit())
    //     .stdout(Stdio::inherit())
    //     .stderr(Stdio::inherit())
    //     .arg("sudo")
    //     .arg("modprobe")
    //     .arg("v4l2loopback")
    //     .arg(format!("video_nr={device_number}"))
    //     .arg(format!("card_label={loopback_label}"))
    //     .arg("exclusive_caps=1")
    //     .output()
    //     .await
    //     .wrap_err("generating loopback device")
    //     .and_then(|out| out.success_output())?;
    let before = list_devices().await?.into_iter().collect::<HashSet<_>>();
    let device_number = video_nr_from_video(&for_video_device)?;
    let loopback_label = loopback_label(&for_video_device)?;
    let as_loopback_device = |device| LoopbackDevice {
        loopback_device: device,
        for_device: for_video_device,
    };

    let to_matching_loopback_device =
        |DetailedVideoDevice {
             video_device,
             details,
         }: &DetailedVideoDevice| {
            details
                .contains(&loopback_label)
                .then(|| video_device.clone())
        };
    if let Some(already_exists) = before.iter().find_map(to_matching_loopback_device) {
        return Ok(as_loopback_device(already_exists));
    }
    Command::new("sudo")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .arg("modprobe")
        .arg("v4l2loopback")
        .arg(format!("video_nr={device_number}"))
        .arg(format!("card_label={loopback_label}"))
        // .arg("exclusive_caps=1")
        .output()
        .await
        .wrap_err("generating loopback device")
        .and_then(|out| out.success_output())?;
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    let after = list_devices().await?.into_iter().collect::<HashSet<_>>();
    after
        .difference(&before)
        .find_map(to_matching_loopback_device)
        .ok_or_else(|| eyre!("video loopback device wasn't created"))
        .map(as_loopback_device)
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
    // apply_video_read_args(&mut command)
    command
        .args([arg!(video_device)])
        .spawn()
        .wrap_err_with(|| format!("spawning ffplay instance for {video_device:?}"))
}

pub static MWCAP_CONTROL_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(Default::default);

#[tracing::instrument]
pub async fn try_enable_low_latency_for_magewell(video_device: VideoDevice) {
    if let Err(magewell_error) = enable_low_latency_for_magewell(video_device).await {
        tracing::error!(?magewell_error);
    }
}

#[instrument(ret, err, level = "info")]
pub async fn enable_low_latency_for_magewell(video_device: VideoDevice) -> Result<()> {
    let _guard = MWCAP_CONTROL_LOCK.lock().await;
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    Command::new("mwcap-control")
        .arg("--video-output-lowlatency")
        .arg("on")
        .arg(video_device.as_ref())
        .output()
        .await
        .wrap_err("spawning command")
        .and_then(|output| output.success_output())
        .and_then(|output| {
            const SUCCESS_MARKER: &str = "successfully";
            [output.stdout.clone(), output.stderr.clone()]
                .join(" ")
                .contains(SUCCESS_MARKER)
                .then_some(())
                .ok_or_else(|| eyre!("output text does not contain word '{SUCCESS_MARKER}'"))
                .wrap_err_with(|| format!("checking output: {output:?}"))
        })
        .wrap_err_with(|| format!("enabling low latency mode for {video_device:?}"))
}

impl FfmpegInstance {
    #[instrument(ret, err)]
    pub async fn new(
        sessions_directory: SessionsDirectory,
        LoopbackDevice {
            loopback_device,
            for_device: video_device,
        }: LoopbackDevice,
        project_name: ProjectName,
        notify: ProcessEventBus,
    ) -> Result<Self> {
        let process_path = "ffmpeg".to_owned();
        try_enable_low_latency_for_magewell(video_device.clone()).await;

        let preview_process = ffplay_preview(loopback_device.clone())
            .map_err(|v| v.wrap_err("spawning ffplay"))
            .and_then(|c| {
                c.gracefully_shutdown_on_drop()
                    .map_err(|e| e.wrap_err("making sure it doesn't quit"))
            })
            .map_ok(|child| ProcessWatcher::new("ffplay".to_owned(), child, notify.clone()))
            .map_err(|v| {
                v.wrap_err(format!(
                    "creating preview window for device {loopback_device}"
                ))
            })
            .map(RwLock::new)
            .map(Arc::new)
            .await;
        ready(video_file_path(sessions_directory, &project_name))
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
                                crate::process::app_interval(std::time::Duration::from_secs(1));

                            loop {
                                interval.tick().await;
                                notify.send(ProcessEvent::NewInput).ok();
                            }
                        })
                        .abort_on_drop();

                        Self {
                            preview_process,
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
        let [file_size_block, ffmpeg_block, ffplay_block]: [Rect; 3] = layout!(Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Ratio(1, 9),
                Constraint::Ratio(5, 9),
                Constraint::Ratio(3, 9),
            ])
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
        self.process.write().render_to_term(f, ffmpeg_block)?;
        self.preview_process
            .write()
            .render_to_term(f, ffplay_block)?;
        Ok(())
    }
}
