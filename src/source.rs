use crate::message::{Message, MessageParseError};
use eventsource_client::{Client, Event, EventStream, HttpsConnector};
use futures::{ready, Stream};
use pin_project::pin_project;
use std::sync::Arc;
use std::{
    convert::TryInto,
    fmt::{Debug, Display},
    pin::Pin,
    task::{Context, Poll},
};

/// default URL for subscribing to the update stream
const DEFAULT_BASE_URL: &str = "https://stream.launchdarkly.com/all";

/// Allows reading a stream of update [Messages](Message)
pub trait Source {
    type Error;
    type Stream: Stream<Item = Result<Message, Self::Error>>;

    /// Get the stream of updates
    ///
    /// Whenever a stream returned an error,
    /// this should be called again to get a
    /// fresh stream.
    fn stream(&self) -> Self::Stream;
}

impl<T: Source> Source for Arc<T> {
    type Error = T::Error;
    type Stream = T::Stream;
    fn stream(&self) -> Self::Stream {
        self.as_ref().stream()
    }
}

/// [Source] for reading from an SSE stream.
///
/// This is the most common protocol LaunchDarkly offers.
pub struct SseSource {
    client: Client<HttpsConnector>,
}

impl SseSource {
    /// Create a [Source] consuming from SSE with an SDK token
    pub fn new<T: AsRef<str>>(token: T) -> Self {
        let client = eventsource_client::Client::for_url(DEFAULT_BASE_URL)
            .unwrap()
            .header("Authorization", token.as_ref())
            .unwrap()
            .build();
        Self { client }
    }
}

impl Source for SseSource {
    type Error = StreamError<eventsource_client::Error>;
    type Stream = MessageStream<Pin<Box<EventStream<HttpsConnector>>>>;

    fn stream(&self) -> Self::Stream {
        MessageStream(Box::pin(self.client.stream()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StreamError<E>
where
    E: Debug + Display,
{
    #[error("Failed to read SSE stream: {0}")]
    Inner(E),

    #[error("Failed to parse event: {0}")]
    Parse(#[from] MessageParseError),
}

/// [Stream] impl for [SseSource]
#[pin_project]
pub struct MessageStream<S>(#[pin] S);

impl<S, E> Stream for MessageStream<S>
where
    S: Stream<Item = Result<Event, E>>,
    E: Debug + Display,
{
    type Item = Result<Message, StreamError<E>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        // poll the stream
        let event = match ready!(this.0.poll_next(cx))
            .transpose()
            .map_err(StreamError::Inner)?
        {
            Some(ev) => ev,
            None => return Poll::Ready(None),
        };
        // convert the event in an update message
        let message = event.try_into()?;
        Poll::Ready(Some(Ok(message)))
    }
}
