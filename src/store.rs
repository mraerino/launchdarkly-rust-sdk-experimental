use crate::{
    consumer::{Consumer, InitState},
    message::{InitData, Message, Update},
    models::FeatureFlagState,
};
use arc_swap::ArcSwap;
use futures::future::{self, Ready};
use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tracing::{info, warn};

pub trait Store {
    fn flag(&self, name: &str) -> Option<FeatureFlagState>;
    fn export_all(&self) -> HashMap<String, FeatureFlagState>;
}

pub struct MemoryStore {
    flags: ArcSwap<HashMap<String, FeatureFlagState>>,
    init: AtomicBool,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        let flags = ArcSwap::new(Arc::new(HashMap::new()));
        Self {
            flags,
            init: AtomicBool::new(false),
        }
    }
}

impl Store for MemoryStore {
    fn flag(&self, name: &str) -> Option<FeatureFlagState> {
        self.flags.load().get(name).cloned()
    }

    fn export_all(&self) -> HashMap<String, FeatureFlagState> {
        self.flags.load().as_ref().clone()
    }
}

impl<T: Store> Store for Arc<T> {
    fn flag(&self, name: &str) -> Option<FeatureFlagState> {
        self.as_ref().flag(name)
    }

    fn export_all(&self) -> HashMap<String, FeatureFlagState> {
        self.as_ref().export_all()
    }
}

impl<S> Consumer<S> for MemoryStore {
    type Error = Infallible;
    type Future = Ready<Result<InitState, Self::Error>>;

    fn consume(&self, msg: Message) -> Self::Future {
        match msg {
            // initialize flag data
            Message::Put(InitData { flags }) => {
                self.flags.store(Arc::new(flags));
                self.init.store(true, Ordering::SeqCst);
            }
            // update a single flag
            Message::Patch(Update::Flag {
                name,
                data: Some(flag),
                ..
            }) => {
                if !self.init.load(Ordering::SeqCst) {
                    warn!("ignoring update sent before init");
                    return future::ready(Ok(InitState::Pending));
                }
                let mut updated = {
                    // Drop once cloned - don't hold guard while storing
                    let flags = self.flags.load();
                    if let Some(existing) = flags.get(&name) {
                        // check that incoming version is newer than what we have
                        if flag.version > existing.version {
                            info!("flag already up-to-date, ignoring");
                            return future::ready(Ok(InitState::Done));
                        }
                    }
                    flags.as_ref().clone()
                };
                updated.insert(name, flag);
                self.flags.store(Arc::new(updated));
            }
            // delete a flag
            Message::Delete(Update::Flag {
                name,
                version: Some(version),
                ..
            }) => {
                if !self.init.load(Ordering::SeqCst) {
                    warn!("ignoring delete sent before init");
                    return future::ready(Ok(InitState::Pending));
                }
                let updated = {
                    // Drop once cloned - don't hold guard while storing
                    let flags = self.flags.load();
                    flags
                        .get(&name)
                        // check that deleted version is newer than what we have
                        .filter(|f| version > f.version)
                        .map(|_| flags.as_ref().clone())
                        .map(|mut f| {
                            f.remove(&name);
                            f
                        })
                };
                if let Some(updated) = updated {
                    self.flags.store(Arc::new(updated));
                }
            }
            msg => {
                warn!(
                    ?msg,
                    "unknown update, missing some info or not yet implemented"
                );
            }
        };
        future::ready(Ok(InitState::Done))
    }
}
