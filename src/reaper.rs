use std::sync::Arc;

use itertools::Itertools;
use tui::{
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{List, ListItem},
};

use super::*;

#[derive(Debug, Clone)]
pub struct ReaperInstance {
    process: Arc<ProcessWatcher>,
    list_state: tui::widgets::ListState,
}

impl ReaperInstance {
    #[instrument(ret, err)]
    pub fn new(project_name: ProjectName, notify: Notify) -> Result<Self> {
        bounded_command("reaper")
            .spawn()
            .wrap_err("spawning process instance")
            .map(|child| ProcessWatcher::new(child, notify))
            .map(Arc::new)
            .map(|process| Self {
                process,
                list_state: Default::default(),
            })
    }
}

impl RenderToTerm for ReaperInstance {
    fn render_to_term<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        let messages = |stdio: Option<&StdioWatcher>| {
            stdio
                .map(|stdio| stdio.inner.read().iter().cloned().collect_vec())
                .unwrap_or_default()
                .into_iter()
        };
        let items = messages(self.process.stdout.as_ref())
            .chain(messages(self.process.stderr.as_ref()))
            .sorted_by_key(|m| m.time)
            .map(|l| l.line);

        let items = items
            .map(|log| ListItem::new(vec![Spans::from(vec![Span::raw(log)])]))
            .collect::<Vec<_>>();
        let items = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Reaper"))
            .highlight_style(
                Style::default()
                    .bg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");
        f.render_stateful_widget(items, rect, &mut self.list_state);

        Ok(())
    }
}
