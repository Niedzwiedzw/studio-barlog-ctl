use super::*;

pub trait ResultZipExt<T, E> {
    fn zip<U>(self, other: Result<U, E>) -> Result<(T, U), E>;
}

impl<T, E> ResultZipExt<T, E> for Result<T, E> {
    fn zip<U>(self, other: Result<U, E>) -> Result<(T, U), E> {
        match (self, other) {
            (Ok(one), Ok(other)) => Ok((one, other)),
            (Ok(_), Err(message)) => Err(message),
            (Err(message), Ok(_)) => Err(message),
            (Err(message), Err(_)) => Err(message),
        }
    }
}

#[macro_export]
macro_rules! layout {
    ($layout:expr) => {
        $layout
            .try_into()
            .map_err(|e| eyre!("invalid layout as [{} {}]: {e:?}", file!(), line!()))
            .expect("bad layout")
    };
}

#[derive(Debug)]
/// this task is no longer being polled when the handle goes out of scope
pub struct AbortOnDrop<T>(pub tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    #[instrument(skip_all)]
    fn drop(&mut self) {
        tracing::trace!("task went out of scope");
        self.0.abort();
    }
}

impl<T> futures::Future for AbortOnDrop<T> {
    type Output = std::result::Result<T, tokio::task::JoinError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.0).poll(cx)
    }
}

pub trait AbortOnDropExt<T> {
    fn abort_on_drop(self) -> AbortOnDrop<T>;
}

impl<T> AbortOnDropExt<T> for tokio::task::JoinHandle<T> {
    fn abort_on_drop(self) -> AbortOnDrop<T> {
        AbortOnDrop(self)
    }
}

pub trait GracefullyShutdownChildExt {
    fn gracefully_shutdown_on_drop(self) -> GracefullyShutdownChild;
}

impl GracefullyShutdownChildExt for tokio::process::Child {
    fn gracefully_shutdown_on_drop(self) -> GracefullyShutdownChild {
        GracefullyShutdownChild(self)
    }
}

#[derive(derive_more::AsMut)]
pub struct GracefullyShutdownChild(tokio::process::Child);

impl Drop for GracefullyShutdownChild {
    fn drop(&mut self) {
        if let Some(pid) = self
            .0
            .id()
            .and_then(|pid| TryInto::<i32>::try_into(pid).ok())
        {
            let pid = nix::unistd::Pid::from_raw(pid);
            if let Err(errno) = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM) {
                tracing::warn!(?errno, "killing the child process failed")
            }
        }
    }
}
