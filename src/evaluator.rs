use crate::{
    models::{fallthrough::Fallthrough, rollout::Rollout, FeatureFlagState},
    store::Store,
};
use hex::ToHex;
use sha1::{Digest, Sha1};
use std::ops::Div;
use tracing::warn;

const BUCKET_DIVIDER: f64 = 0xFFFFFFFFFFFFFFFu64 as f64;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Requested flag was not found")]
    FlagNotFound,

    #[error("Flag is off")]
    FlagOff,

    #[error("Prerequisite did not match")]
    PrerequisiteFailed,

    #[error("Prerequisite was invalid")]
    InvalidPrerequisite,

    #[error("Target was invalid")]
    InvalidTarget,

    #[error("Malformed variations in rollout")]
    InvalidRollout,

    #[error("Evaluation of rules is not supported right now")]
    UnsupportedRules,

    #[error("Fallthrough is expected to either have a fixed variation or a rollout")]
    EmptyFallthrough,

    #[error("Invalid variation: Index not in range")]
    IndexOutOfRange,

    #[error("Type of variation is invalid")]
    InvalidVariationType,
}

/// Represents a user
///
/// Has a single key right now
#[derive(Debug)]
pub struct User<'a> {
    key: &'a str,
    // todo: Support additional attributes (key-value)
}

impl<'a> User<'a> {
    /// Create a user based on a key
    pub fn new(key: &'a str) -> Self {
        Self { key }
    }
}

/// Used to evaluate flags by reading from a [Store]
/// and running the [flag algorithm](https://docs.launchdarkly.com/sdk/concepts/flag-evaluation-rules).
pub struct Evaluator<S> {
    store: S,
}

/// Helper for a single evaluation
///
/// Contains the actual evaluation implementation
pub struct Evaluation<'a, 'u, S> {
    flag: &'a FeatureFlagState,
    user: &'a User<'u>,
    store: &'a S,
}

impl<'a, 'u, S: Store> Evaluation<'a, 'u, S> {
    /// Create an evaluation from a store, a flag, a user
    ///
    /// The store is required to fetch more flags in the
    /// prerequisites step.
    pub fn new(store: &'a S, flag: &'a FeatureFlagState, user: &'a User<'u>) -> Self {
        Self { flag, user, store }
    }

    /// Runs the evaluation algorithm and returns the correct
    /// variation value.
    ///
    /// Returns a [json `Value` enum](serde_json::Value) which should
    /// be tried to cast into the desired type
    pub fn run(&self) -> Result<serde_json::Value, Error> {
        let index = self.index()?;

        let variation = self
            .flag
            .variations
            .get(index)
            .ok_or(Error::IndexOutOfRange)?
            .clone();
        Ok(variation)
    }

    /// Find the variation index for this evaluation
    ///
    /// Runs the evaluation algortihm described here:
    /// https://docs.launchdarkly.com/sdk/concepts/flag-evaluation-rules
    ///
    /// The returned number can be used as an index into the variations
    /// of a flag.
    //
    // todo: Return a reason with the result
    fn index(&self) -> Result<usize, Error> {
        // Preliminary checks
        // https://docs.launchdarkly.com/sdk/concepts/flag-evaluation-rules#preliminary-checks
        if self.user.key.is_empty() {
            warn!("User key is empty");
        }
        if !self.flag.on {
            return Ok(self.flag.off_variation);
        }

        if self.prerequisites().is_err() {
            return Ok(self.flag.off_variation);
        }

        if let Some(target_variation) = self.targets()? {
            return Ok(target_variation as usize);
        }

        if let Some(rule_variation) = self.rules()? {
            return Ok(rule_variation as usize);
        }

        self.fallthrough().map(|v| v as usize)
    }

    /// Checks prerequesite flags
    ///
    /// https://docs.launchdarkly.com/sdk/concepts/flag-evaluation-rules#prerequisite-checks
    fn prerequisites(&self) -> Result<(), Error> {
        for prereq in &self.flag.prerequisites {
            // get flag name and expected variation index
            let (key, expected) = prereq
                .key
                .as_ref()
                .and_then(|k| prereq.variation.map(|v| (k, v)))
                .ok_or(Error::InvalidPrerequisite)?;
            // retrieve flag
            let flag = self.store.flag(key).ok_or(Error::FlagNotFound)?;
            if !flag.on {
                return Err(Error::FlagOff);
            }
            // compute variation index for the flag
            let index = Evaluation::new(self.store, &flag, self.user).index()? as i64;
            if index != expected {
                // short-circuit once the first value differs
                return Err(Error::PrerequisiteFailed);
            }
        }
        Ok(())
    }

    /// Checks individual target matches
    ///
    /// https://docs.launchdarkly.com/sdk/concepts/flag-evaluation-rules#individual-targeting-checks
    fn targets(&self) -> Result<Option<i64>, Error> {
        for target in &self.flag.targets {
            // look at a target variation
            let (values, variation) = target
                .values
                .as_ref()
                .and_then(|vals| target.variation.map(|v| (vals, v)))
                .ok_or(Error::InvalidTarget)?;
            for value in values {
                if value == self.user.key {
                    // return variation if matches user
                    return Ok(Some(variation));
                }
            }
        }
        Ok(None)
    }

    /// Checks rule matches
    ///
    /// UNSUPPORTED right now.
    /// Will return an error if the flag has rules.
    ///
    /// https://docs.launchdarkly.com/sdk/concepts/flag-evaluation-rules#targeting-rule-checks
    fn rules(&self) -> Result<Option<i64>, Error> {
        // TODO: Support rule matching
        if !self.flag.rules.is_empty() {
            return Err(Error::UnsupportedRules);
        }
        Ok(None)
    }

    /// Determine falltrough variation
    ///
    /// https://docs.launchdarkly.com/sdk/concepts/flag-evaluation-rules#fallthrough
    ///
    /// Fails if neither single variation nor rollout present
    fn fallthrough(&self) -> Result<i64, Error> {
        let Fallthrough { variation, rollout } = &self.flag.fallthrough;

        // simple route: single fallthrough variation
        if let Some(variation) = variation {
            return Ok(*variation);
        }

        // advanced: percentage-based rollout
        self.rollout(rollout.as_ref().ok_or(Error::EmptyFallthrough)?)
    }

    /// Determine variation based on a Rollout
    ///
    /// Matches a consistent user bucket against rollout segments.
    /// Each rollout segment has a relative value.
    /// With correct data they add up to 100%.
    ///
    /// https://docs.launchdarkly.com/sdk/concepts/flag-evaluation-rules#rollouts
    fn rollout(&self, rollout: &Rollout) -> Result<i64, Error> {
        let variations = rollout
            .variations
            .as_ref()
            .filter(|v| !v.is_empty())
            .ok_or(Error::InvalidRollout)?;

        // compute user bucket (relative value: 0-1)
        let bucket = self.bucket();

        let mut sum = 0f64;
        for variation in variations {
            let weight = variation.weight.ok_or(Error::InvalidRollout)? as f64;
            // accumulate relative weights
            // stored as num 0 - 100_000 in config
            // scaled to 0-1 to match bucket range
            let add = weight / 100_000f64;
            sum += add;

            // user matches when passing bucket threshold
            if bucket < sum {
                return variation.variation.ok_or(Error::InvalidRollout);
            }
        }

        // would be caused by data inconsistency
        // only happens if the rollout weights do not add up to 100%
        Err(Error::InvalidRollout)
    }

    /// Determine the rollout bucket for the current user
    ///
    /// https://docs.launchdarkly.com/sdk/concepts/flag-evaluation-rules#rollouts
    fn bucket(&self) -> f64 {
        // todo: support a custom user attribute
        // todo: support the secondary user identifier

        // compute SHA1 hash for user from flag, salt & user
        let hash = &Sha1::new()
            .chain(&self.flag.key)
            .chain(".")
            .chain(&self.flag.salt)
            .chain(".")
            .chain(self.user.key)
            .finalize()[..];
        // hex string of the hash is cut to first 15 characters
        let mut hex: String = hash.encode_hex();
        hex.truncate(15);
        // convert to u64
        let val = u64::from_str_radix(&hex, 16).unwrap() as f64;

        // divide by const, results in value between 0 and 1
        val.div(BUCKET_DIVIDER)
    }
}

pub trait Evaluate {
    /// Determines the variation value for a flag
    ///
    /// Returns a json value enum which can be casted into the desired type
    fn evaluate(&self, flag: &str, user: &User) -> Result<serde_json::Value, Error>;

    /// Determine a bool flag variation value
    ///
    /// Recommended to use the result with `.unwrap_or` to always get a value
    fn bool_variation(&self, flag: &str, user: &User) -> Result<bool, Error> {
        self.evaluate(flag, user)?
            .as_bool()
            .ok_or(Error::InvalidVariationType)
    }
}

impl<S: Store> Evaluator<S> {
    /// Create an evaluator for a [Store]
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl<S: Store> Evaluate for Evaluator<S> {
    fn evaluate(&self, flag: &str, user: &User) -> Result<serde_json::Value, Error> {
        // get flag from store
        let flag = self.store.flag(flag).ok_or(Error::FlagNotFound)?;
        // find variation based on rules
        Evaluation::new(&self.store, &flag, user).run()
    }
}

#[cfg(test)]
mod tests {
    use super::{Evaluation, User};
    use crate::test_utils::{FlagBuilder, MockStore};

    fn setup() -> (User<'static>, MockStore) {
        let user = User::new("test-user");
        let store = MockStore::new();
        (user, store)
    }

    #[test]
    fn fallthrough() {
        let (user, mut store) = setup();
        let flag = FlagBuilder::default()
            .on()
            .with_key("eval_test")
            .with_fallthrough_variation(1)
            .into_inner();
        store.add(flag.clone());
        let eval = Evaluation::new(&store, &flag, &user);
        assert_eq!(1, eval.index().expect("failed to get variation index"));
    }

    #[test]
    fn fallthrough_rollout() {
        let (user1, mut store) = setup();
        let flag = FlagBuilder::default()
            .on()
            .with_key("eval_test")
            // 30/70 % split
            .with_fallthrough_rollout(vec![(0, 30000), (1, 70000)])
            .into_inner();
        store.add(flag.clone());

        let eval = Evaluation::new(&store, &flag, &user1);
        assert_eq!(1, eval.index().expect("failed to get variation index"));

        let user2 = User::new("my-other-user");
        let eval = Evaluation::new(&store, &flag, &user2);
        assert_eq!(0, eval.index().expect("failed to get variation index"));
    }

    #[test]
    fn targeting() {
        let (user, mut store) = setup();
        let flag = FlagBuilder::default()
            .on()
            .with_key("eval_test")
            .add_target(1, "test-user")
            .into_inner();
        store.add(flag.clone());

        let eval = Evaluation::new(&store, &flag, &user);
        assert_eq!(1, eval.index().expect("failed to get variation index"));
    }
}
