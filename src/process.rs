use std::collections::VecDeque;

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
    notify: Notify,
}

impl StdioWatcher {
    pub const MESSAGE_CAPACITY: usize = 200;
    pub fn new(notify: Notify) -> Self {
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
                move |message| notify.send(()).map(|_| message).ok()
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
    child: tokio::process::Child,
    pub stdout: Option<StdioWatcher>,
    pub stderr: Option<StdioWatcher>,
}

impl ProcessWatcher {
    pub fn new(mut child: tokio::process::Child, notify: Notify) -> Self {
        Self {
            stdout: child
                .stdout
                .take()
                .map(|stdout| StdioWatcher::new(notify.clone()).watching(stdout)),
            stderr: child
                .stderr
                .take()
                .map(|stderr| StdioWatcher::new(notify.clone()).watching(stderr)),
            child,
        }
    }
}
