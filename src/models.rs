//! Contains models generated from the LaunchDarkly OpenAPI spec
//! at https://github.com/launchdarkly/ld-openapi/blob/master/spec/definitions.yaml
//!
//! See the `build.rs` for details.

#![allow(dead_code)]
#![allow(clippy::useless_conversion)]
#![allow(clippy::wrong_self_convention)]
#![allow(clippy::should_implement_trait)]

include!(concat!(env!("OUT_DIR"), "/models/mod.rs"));

use self::{
    client_side_availability::ClientSideAvailability, fallthrough::Fallthrough,
    prerequisite::Prerequisite, rule::Rule, target::Target,
};
use serde::Deserialize;

/// Special struct for deserializing SSE updates.
///
/// This struct is not present in the OpenAPI spec,
/// but uses some of the generated models for its fields.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct FeatureFlagState {
    #[serde(rename = "clientSide")]
    pub client_side: bool,
    #[serde(rename = "clientSideAvailability")]
    pub client_side_availability: ClientSideAvailability,
    pub deleted: bool,
    pub fallthrough: Fallthrough,
    pub key: String,
    #[serde(rename = "offVariation")]
    pub off_variation: usize,
    pub on: bool,
    pub prerequisites: Vec<Prerequisite>,
    pub rules: Vec<Rule>,
    pub salt: String,
    pub targets: Vec<Target>,
    #[serde(rename = "trackEvents")]
    pub track_events: bool,
    #[serde(rename = "trackEventsFallthrough")]
    pub track_events_fallthrough: bool,
    pub variations: Vec<serde_json::Value>,
    pub version: u64,
}
