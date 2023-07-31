use std::collections::VecDeque;

use tui::{
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{List, ListItem},
};

use super::*;

#[derive(Debug, Clone)]
pub struct StdioMessage {
    pub time: ProjectTime,
    pub line: String,
}

impl StdioMessage {
    pub fn new(line: String) -> Self {
        Self {
            time: crate::now(),
            line,
        }
    }
}

#[derive(Clone, Debug)]
pub struct StdioWatcher {
    pub inner: Arc<RwLock<VecDeque<StdioMessage>>>,
    notify: ProcessEventBus,
}

impl StdioWatcher {
    pub const MESSAGE_CAPACITY: usize = 200;
    pub fn new(notify: ProcessEventBus) -> Self {
        Self {
            inner: Arc::new(RwLock::new(VecDeque::with_capacity(Self::MESSAGE_CAPACITY))),
            notify,
        }
    }
    pub fn watching<T: AsyncRead + Unpin + Send + 'static>(self, reader: T) -> Self {
        let inner = self.inner.clone();
        let notify = self.notify.clone();
        tokio::task::spawn(async move {
            let reader = BufReader::new(reader);
            let mut reader = reader.lines();
            let notify = {
                to_owned![notify];
                move |message| notify.send(ProcessEvent::NewInput).map(|_| message).ok()
            };
            while let Some(line) = reader
                .next_line()
                .await
                .ok()
                .and_then(|v| v)
                .and_then(notify.clone())
            {
                let mut inner = inner.write();
                if inner.len() > Self::MESSAGE_CAPACITY {
                    inner.pop_back();
                }
                inner.push_front(StdioMessage::new(line));
            }
        });
        self
    }
}

#[derive(Debug)]
pub struct ProcessWatcher {
    pub name: String,
    pub stdout: Option<StdioWatcher>,
    pub stderr: Option<StdioWatcher>,
    pub status: Arc<RwLock<Option<String>>>,
}

#[derive(Debug)]
pub enum ProcessStatus {
    Running,
    Exited(String),
}

impl ProcessWatcher {
    pub fn new(name: String, mut child: tokio::process::Child, notify: ProcessEventBus) -> Self {
        let status = Arc::new(RwLock::new(None));
        let stdout = child
            .stdout
            .take()
            .map(|stdout| StdioWatcher::new(notify.clone()).watching(stdout));
        let stderr = child
            .stderr
            .take()
            .map(|stderr| StdioWatcher::new(notify.clone()).watching(stderr));

        {
            to_owned![notify, status];
            tokio::task::spawn(async move {
                let res = child.wait().await;
                let _ = status.write().insert(format!("{res:?}"));
                if let Err(message) = notify
                    .send(ProcessEvent::ProcessExtied)
                    .wrap_err("notifying of process exit")
                {
                    tracing::error!(?message);
                }
            });
        }
        Self {
            status,
            name,
            stdout,
            stderr,
        }
    }

    /// returns None if process is still running
    pub fn status(&mut self) -> ProcessStatus {
        match self.status.read().as_ref() {
            Some(exit) => ProcessStatus::Exited(exit.to_owned()),
            None => ProcessStatus::Running,
        }
    }
}

impl RenderToTerm for ProcessWatcher {
    fn render_to_term<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        let title = format!(
            "{}{}",
            self.name.clone(),
            match self.status() {
                ProcessStatus::Running => "".to_owned(),
                ProcessStatus::Exited(message) => format!(" (exited: {message})"),
            }
        );
        let messages = |stdio: Option<&StdioWatcher>| {
            stdio
                .map(|stdio| stdio.inner.read().iter().cloned().collect_vec())
                .unwrap_or_default()
                .into_iter()
        };
        let items = messages(self.stdout.as_ref())
            .chain(messages(self.stderr.as_ref()))
            .sorted_by_key(|m| m.time)
            .map(|l| l.line);

        let items = items
            .map(|log| ListItem::new(vec![Spans::from(vec![Span::raw(log)])]))
            .collect::<Vec<_>>();
        let items = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(
                Style::default()
                    .bg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");
        f.render_widget(items, rect);
        Ok(())
    }
}
