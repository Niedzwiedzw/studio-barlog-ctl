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
}

impl StudioState {
    pub fn new(project_name: ProjectName, template: PathBuf) -> Result<Self> {
        let (notify, wake_up) = tokio::sync::mpsc::unbounded_channel();
        let qpwgraph =
            crate::qpwgraph::QpwgraphInstance::new(notify.clone()).wrap_err("Spawning qpwgraph")?;
        let reaper = crate::reaper::ReaperInstance::new(project_name, template, notify.clone())
            .wrap_err("starting reaper")?;
        Ok(Self {
            wake_up: Some(UnboundedReceiverStream::new(wake_up)),
            reaper,
            qpwgraph,
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
        } = self;
        macro_rules! layout {
            ($layout:expr) => {
                $layout
                    .try_into()
                    .map_err(|e| eyre!("invalid layout as [{} {}]: {e:?}", file!(), line!()))
                    .expect("bad layout")
            };
        }
        let [header, body]: [Rect; 2] = layout!(Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(10), Constraint::Percentage(90)].as_ref())
            .split(rect));
        let [qpwgraph_col, reaper_col, _]: [Rect; 3] = layout!(Layout::default()
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
        reaper.render_to_term(frame, reaper_col)?;
        qpwgraph.render_to_term(frame, qpwgraph_col)?;

        Ok(())
    }
}
