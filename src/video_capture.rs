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

impl FfmpegInstance {
    #[instrument(ret, err)]
    pub fn new(project_name: ProjectName, notify: ProcessEventBus) -> Result<Self> {
        let process_path = "ffmpeg".to_owned();
        const RATE: usize = 25;
        const VIDEO_SIZE: &str = "1920x1080";
        const VIDEO_DEVICE: &str = "/dev/video1";

        macro_rules! arg {
            ($arg:expr) => {
                format!("{}", $arg).as_str()
            };
        }

        video_file_path(&project_name).and_then(|video_file_path| {
            bounded_command(&process_path)
                .args(["-thred_queue_size", arg!(512)])
                .args(["-r", arg!(RATE)])
                .args(["-f", arg!("v412")])
                .args(["-video_size", arg!(VIDEO_SIZE)])
                .args(["-i", arg!(VIDEO_DEVICE)])
                .args(["-crf", arg!(0)])
                .args(["-c:v", arg!("lib264")])
                .args(["-preset", arg!("ultrafast")])
                .args(["-threads", arg!(8)])
                .arg(&video_file_path)
                .spawn()
                .wrap_err("spawning ffmpeg instance")
                .map(|child| ProcessWatcher::new(process_path, child, notify))
                .map(RwLock::new)
                .map(Arc::new)
                .map(|process| Self {
                    process,
                    video_file_path,
                })
        })
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
        self.process.write().render_to_term(f, rect)?;
        Ok(())
    }
}
