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
