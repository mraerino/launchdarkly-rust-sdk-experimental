use crate::{message::Message, source::Source};
use futures::{future::BoxFuture, Future, FutureExt, StreamExt};
use std::{error::Error as StdError, fmt, sync::Arc};
use tokio::{sync::watch, task};
use tracing::warn;

#[derive(Clone, Debug, thiserror::Error)]
pub enum ReadError<E>
where
    E: Clone + fmt::Debug + StdError + 'static,
{
    #[error("Background task stopped before sending result")]
    TaskDropped,

    #[error("Starting stream failed 4 times in a row")]
    RetryFailed,

    #[error(transparent)]
    Inner(#[from] E),
}

/// Represents the state of a [Consumer]
/// after consuming a message
pub enum InitState {
    Pending,
    Done,
}

/// A Consumer reads messages from a source and persists them
///
/// Should be implemented for any [Store](crate::store::Store)
/// when intended for prod
pub trait Consumer<S> {
    type Error;
    type Future: Future<Output = Result<InitState, Self::Error>> + Send;

    /// Process a single message coming from a [Source]
    ///
    /// Receives a unique reference only, so it stays portable and
    /// queries on stores can be made concurrently.
    /// Use atomic updates or an inner mutex to mutate.
    fn consume(&self, msg: Message) -> Self::Future;

    /// Start reading messages from a stream and provide readiness signaling
    /// and retries.
    ///
    /// Usually just wraps [`consume`] in a background task.
    ///
    /// Default impl will abort after 4 consecutive stream failures.
    /// Waits until the consumer got the init data (transitioned to InitState::Done).
    ///
    /// When not interested in readiness, just drop the returned future. This has no
    /// bad consequences.
    fn read_from(
        self: Arc<Self>,
        source: S,
    ) -> BoxFuture<'static, Result<(), ReadError<Self::Error>>>
    where
        Self: Send + Sync + 'static,
        Self::Error: fmt::Debug + StdError + Clone + Sync + Send,
        S: Source + Send + 'static,
        S::Stream: Unpin + Send,
        S::Error: fmt::Display + Send,
    {
        let (init_tx, mut init_rx) =
            watch::channel::<Option<Result<(), ReadError<Self::Error>>>>(None);

        task::spawn(async move {
            let mut stream = source.stream();
            let mut failures = 0;
            while failures < 4 {
                let msg = match stream.next().await {
                    Some(Ok(msg)) => msg,
                    Some(Err(error)) => {
                        failures += 1;
                        warn!(%error, "failed processing event, restarting stream");
                        // TODO: consider exponential backoff
                        // retry stream (usually reopens the connection)
                        stream = source.stream();
                        continue;
                    }
                    None => return,
                };
                // reset failure counter after single successful read
                failures = 0;

                match self.consume(msg).await {
                    Err(e) => {
                        let _ = init_tx.send(Some(Err(e.into())));
                    }
                    Ok(InitState::Done) => {
                        let _ = init_tx.send(Some(Ok(())));
                    }
                    Ok(InitState::Pending) => {}
                };
            }

            // Exited loop after too many failures
            let _ = init_tx.send(Some(Err(ReadError::RetryFailed)));
        });

        // future to wait for readiness
        async move {
            if init_rx.borrow().is_none() {
                init_rx
                    .changed()
                    .await
                    .map_err(|_| ReadError::TaskDropped)?;
            }
            // safe to unwrap: if it's still None at this point, it's a bug
            let res = init_rx.borrow().as_ref().cloned().unwrap();
            res
        }
        .boxed()
    }
}
