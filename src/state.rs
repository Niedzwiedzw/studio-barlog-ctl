use tokio_stream::wrappers::UnboundedReceiverStream;
use tui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
};

use super::*;

pub struct StudioState {
    pub wake_up: Option<UnboundedReceiverStream<ProcessEvent>>,
    reaper: ReaperInstance,
    qpwgraph: QpwgraphInstance,
    ffmpeg: FfmpegInstance,
}

impl StudioState {
    pub async fn new(
        project_name: ProjectName,
        template: PathBuf,
        reaper_web_base_url: reqwest::Url,
        video_device: VideoDevice,
    ) -> Result<Self> {
        let (notify, wake_up) = tokio::sync::mpsc::unbounded_channel();
        let qpwgraph = crate::qpwgraph::QpwgraphInstance::new(notify.clone())
            .await
            .wrap_err("Spawning qpwgraph")?;

        let (reaper, ffmpeg) = futures::future::try_join(
            crate::reaper::ReaperInstance::new(
                project_name.clone(),
                template,
                notify.clone(),
                reaper_web_base_url,
            )
            .map(|v| v.wrap_err("starting reaper")),
            FfmpegInstance::new(video_device, project_name.clone(), notify.clone())
                .map(|v| v.wrap_err("spawning ffmpeg")),
        )
        .await?;
        Ok(Self {
            wake_up: Some(UnboundedReceiverStream::new(wake_up)),
            reaper,
            qpwgraph,
            ffmpeg,
        })
    }
}

impl crate::rendering::RenderToTerm for StudioState {
    fn render_to_term<B: Backend>(
        &mut self,
        frame: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        let Self {
            wake_up: _,
            reaper,
            qpwgraph,
            ffmpeg,
        } = self;
        let [header, body]: [Rect; 2] = layout!(Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(10), Constraint::Percentage(90)].as_ref())
            .split(rect));
        let [qpwgraph_col, reaper_col, ffmpeg_frame]: [Rect; 3] = layout!(Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ])
            .split(body));

        frame.render_widget(
            Block::default()
                .title(Span::styled(
                    clap::crate_name!(),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL),
            header,
        );
        qpwgraph.render_to_term(frame, qpwgraph_col)?;
        reaper.render_to_term(frame, reaper_col)?;
        ffmpeg.render_to_term(frame, ffmpeg_frame)?;

        Ok(())
    }
}
