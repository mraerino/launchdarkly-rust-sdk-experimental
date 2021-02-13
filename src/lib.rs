use self::{
    consumer::{Consumer, ReadError},
    evaluator::Evaluator,
    source::{Source, SseSource},
    store::{MemoryStore, Store},
};
use evaluator::Evaluate;
use http::header::InvalidHeaderValue;
use models::FeatureFlagState;
use std::{collections::HashMap, error::Error as StdError, fmt, sync::Arc};

pub mod consumer;
pub mod evaluator;
pub mod message;
pub mod models;
pub mod source;
pub mod store;
#[cfg(test)]
mod test_utils;

#[derive(Debug, thiserror::Error)]
pub enum StartError<CE>
where
    CE: fmt::Debug + Clone + StdError + 'static,
{
    #[error("Already started, can't start multiple times")]
    AlreadyStarted,

    #[error("Failed to start reading from source: {0}")]
    Start(#[from] ReadError<CE>),
}

#[derive(Debug, thiserror::Error)]
pub enum CreateError {
    #[error("Invalid SDK token: {0}")]
    InvalidToken(InvalidHeaderValue),
}

/// Client providing the idiomatic way of retrieving
/// variation values for flags.
///
/// Glue code on top of the smaller building blocks.
pub struct DefaultClient<ST, SRC> {
    store: Arc<ST>,
    evaluator: Evaluator<Arc<ST>>,
    source: Option<SRC>,
}

impl DefaultClient<MemoryStore, SseSource> {
    /// Create a feature flagging client based on an SDK token.
    pub fn with_token(token: String) -> Result<Self, CreateError> {
        let source = SseSource::new(&token);
        let store = Arc::new(MemoryStore::new());
        Ok(Self::new(store, source))
    }
}

impl<ST, SRC> DefaultClient<ST, SRC>
where
    ST: Store,
{
    /// Make a client with custom components
    pub fn new<STA: Into<Arc<ST>>>(store: STA, source: SRC) -> Self {
        let store = store.into();
        let evaluator = Evaluator::new(Arc::clone(&store));
        Self {
            evaluator,
            store,
            source: Some(source),
        }
    }

    /// Start consuming data in the client
    ///
    /// Future resolves once the initial data has been read.
    /// Drop the future to ignore the startup. It will still
    /// happen in the background.
    pub async fn start(&mut self) -> Result<(), StartError<ST::Error>>
    where
        ST: Consumer<SRC> + Send + Sync + 'static,
        ST::Error: StdError + Clone + Send + Sync,
        SRC: Source + Send + 'static,
        SRC::Stream: Unpin + Send,
        SRC::Error: StdError + Send,
    {
        let source = self.source.take().ok_or(StartError::AlreadyStarted)?;
        let store = Arc::clone(&self.store);
        store.read_from(source).await.map_err(Into::into)
    }

    /// Export the feature flagging data from the underlying [Store]
    pub fn export(&self) -> HashMap<String, FeatureFlagState> {
        self.store.export_all()
    }
}

impl<ST, SRC> Evaluate for DefaultClient<ST, SRC>
where
    ST: Store,
{
    fn evaluate(
        &self,
        flag: &str,
        user: &evaluator::User,
    ) -> Result<serde_json::Value, evaluator::Error> {
        self.evaluator.evaluate(flag, user)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        evaluator::{Evaluate, User},
        test_utils::{FlagBuilder, MockStore, NullSource},
        DefaultClient,
    };

    #[tokio::test]
    async fn smoke() {
        let mut store = MockStore::new();
        let flag = FlagBuilder::default()
            .on()
            .with_key("smoke_flag")
            .add_target(1, "kalk.space")
            .add_target(1, "www.netlify.com")
            .into_inner();
        store.add(flag);

        let source = NullSource {};
        let client = DefaultClient::new(store, source);

        {
            let user = User::new("kalk.space");
            let result = client
                .bool_variation("smoke_flag", &user)
                .expect("evaluation failed");
            assert!(result);
        }
        {
            let user = User::new("app.netlify.com");
            let result = client
                .bool_variation("smoke_flag", &user)
                .expect("evaluation failed");
            assert!(!result);
        }
    }
}
