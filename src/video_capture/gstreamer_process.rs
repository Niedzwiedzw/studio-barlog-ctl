use super::*;
use tokio_util::sync::CancellationToken;
pub mod low_level;

#[derive(Debug)]
pub struct GstreamerInstance {
    pub video_device: VideoDevice,
    pub video_file_path: PathBuf,
    cancel: CancellationToken,
    _process: AbortOnDrop<Result<()>>,
    _file_size_updater: AbortOnDrop<()>,
}

impl GstreamerInstance {
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
    pub async fn new(
        video_device: VideoDevice,
        output_file_path: PathBuf,
        notify: ProcessEventBus,
    ) -> Result<Self> {
        let cancel = CancellationToken::new();
        let process = {
            to_owned![cancel, video_device, output_file_path];
            tokio::task::spawn_blocking(move || {
                low_level::start_stream(
                    video_device.clone(),
                    output_file_path.clone(),
                    cancel.clone(),
                )
            })
            .abort_on_drop()
        };
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        if process.0.is_finished() {
            match process.await {
                Ok(_) => {
                    bail!("this should never happen - process just stopped recording for no reason")
                }
                Err(report) => return Err(eyre!("{report}")).and_then(|v| v),
            }
        }
        let file_size_updater = tokio::task::spawn(async move {
            let mut interval = crate::process::app_interval(std::time::Duration::from_secs(1));

            loop {
                interval.tick().await;
                notify.send(ProcessEvent::NewInput).ok();
            }
        })
        .abort_on_drop();
        Ok(Self {
            video_device,
            video_file_path: output_file_path,
            cancel,
            _process: process,
            _file_size_updater: file_size_updater,
        })
    }
}

impl Drop for GstreamerInstance {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl RenderToTerm for GstreamerInstance {
    fn render_to_term<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        let [file_size_block, gstreamer_block]: [Rect; 2] = layout!(Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Ratio(5, 9), Constraint::Ratio(4, 9),])
            .split(rect));
        let text_block = |text: String, title: String| {
            let block = Block::default().borders(Borders::ALL).title(Span::styled(
                title,
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
            Paragraph::new(text).block(block).wrap(Wrap { trim: false })
        };

        f.render_widget(
            text_block(
                format!(
                    "running: {}",
                    !(self.cancel.is_cancelled() || self._process.0.is_finished())
                ),
                format!("GStreamer ({:?})", self.video_device),
            ),
            gstreamer_block,
        );
        f.render_widget(
            text_block(
                self.file_size()
                    .unwrap_or_else(|e| format!("reading size: {e:?}")),
                format!("file size ({})", self.video_file_path.display()),
            ),
            file_size_block,
        );

        Ok(())
    }
}
