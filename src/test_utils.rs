use crate::{
    message::Message,
    models::{
        fallthrough::Fallthrough, rollout::Rollout, target::Target,
        weighted_variation::WeightedVariation, FeatureFlagState,
    },
    source::Source,
    store::Store,
};
use std::{collections::HashMap, convert::Infallible};

pub struct MockStore {
    flags: HashMap<String, FeatureFlagState>,
}

impl MockStore {
    pub fn new() -> Self {
        Self {
            flags: HashMap::new(),
        }
    }

    pub fn add(&mut self, flag: FeatureFlagState) {
        self.flags.insert(flag.key.clone(), flag);
    }
}

impl Store for MockStore {
    fn flag(&self, name: &str) -> Option<FeatureFlagState> {
        self.flags.get(name).cloned()
    }

    fn export_all(&self) -> HashMap<String, FeatureFlagState> {
        self.flags.clone()
    }
}

pub struct NullSource;

impl Source for NullSource {
    type Error = Infallible;
    type Stream = futures::stream::Pending<Result<Message, Self::Error>>;

    fn stream(&self) -> Self::Stream {
        futures::stream::pending()
    }
}

pub struct FlagBuilder(FeatureFlagState);

impl Default for FlagBuilder {
    fn default() -> Self {
        Self(FeatureFlagState {
            fallthrough: Fallthrough::builder().variation(0).into(),
            key: "my_test_flag".into(),
            off_variation: 0,
            on: true,
            salt: "test-salt".into(),
            variations: vec![false.into(), true.into()],
            ..Default::default()
        })
    }
}

#[allow(dead_code)]
impl FlagBuilder {
    pub fn off(mut self) -> Self {
        self.0.on = false;
        self
    }

    pub fn on(mut self) -> Self {
        self.0.on = true;
        self
    }

    pub fn with_key<K: Into<String>>(mut self, key: K) -> Self {
        self.0.key = key.into();
        self
    }

    pub fn with_variations<I: IntoIterator<Item = V>, V: Into<serde_json::Value>>(
        mut self,
        iter: I,
    ) -> Self {
        self.0.variations = iter.into_iter().map(|v| v.into()).collect();
        self
    }

    pub fn with_fallthrough_variation(mut self, idx: usize) -> Self {
        self.0.fallthrough = Fallthrough {
            variation: Some(idx as i64),
            ..Default::default()
        };
        self
    }

    pub fn with_fallthrough_rollout<I: IntoIterator<Item = (u32, u32)>>(
        mut self,
        variations: I,
    ) -> Self {
        let variations = variations
            .into_iter()
            .map(|(v, w)| WeightedVariation::builder().variation(v).weight(w).into());
        let rollout = Rollout::builder().variations(variations).into();
        self.0.fallthrough = Fallthrough::builder().rollout(rollout).into();
        self
    }

    pub fn clear_targets(mut self) -> Self {
        self.0.targets = Default::default();
        self
    }

    pub fn add_target<V: Into<String>>(mut self, variation: u32, value: V) -> Self {
        if let Some(target) = self
            .0
            .targets
            .iter_mut()
            .find(|t| t.variation == Some(variation as i64))
        {
            let mut values = target.values.take().unwrap_or_default();
            values.push(value.into());
            target.values.replace(values);
        } else {
            self.0.targets.push(
                Target::builder()
                    .variation(variation)
                    .values([value.into()].iter())
                    .into(),
            );
        }
        self
    }

    pub fn into_inner(self) -> FeatureFlagState {
        self.0
    }
}
