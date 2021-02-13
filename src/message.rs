use crate::models::FeatureFlagState;
use eventsource_client::Event;
use serde::Deserialize;
use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    path::PathBuf,
};
use tracing::{trace, warn};

#[derive(Debug, thiserror::Error)]
pub enum MessageParseError {
    #[error("Failed to parse put data: {0}")]
    ParsePut(serde_json::Error),

    #[error("Missing the data field")]
    MissingData,

    #[error("Missing payload on eventsource item")]
    MissingEventPayload,

    #[error(transparent)]
    ParsePatch(#[from] FromPatchDataError),

    #[error("Unable to parse event payload: {0}")]
    ParsePayload(serde_json::Error),
}

/// Parsed message from the stream
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Message {
    Put(InitData),
    Patch(Update),
    Delete(Update),
    Unknown,
}

impl TryFrom<Event> for Message {
    type Error = MessageParseError;

    fn try_from(event: Event) -> Result<Self, Self::Error> {
        // require an event name
        let name = &event.event_type;
        trace!(%name, "reading SSE event");

        // parse event json
        let event_data = event
            .field("data")
            .ok_or(MessageParseError::MissingEventPayload)?;
        let payload: MessagePayload =
            serde_json::from_slice(event_data).map_err(MessageParseError::ParsePayload)?;

        match name.as_str() {
            "put" => {
                let data = payload.data.ok_or(MessageParseError::MissingData)?;
                // parse into specific struct
                let flag_config: InitData =
                    serde_json::from_value(data).map_err(MessageParseError::ParsePut)?;
                trace!(num_flags = flag_config.flags.len(), "parsed init data");
                Ok(Self::Put(flag_config))
            }
            // change or delete a single record
            "patch" | "delete" => {
                // convert to path-based update
                let update: Update = payload.try_into()?;
                trace!(?update, "parsed update");
                Ok(match name.as_str() {
                    "patch" => Self::Patch(update),
                    "delete" => Self::Delete(update),
                    _ => unreachable!(),
                })
            }
            // unknown
            _ => {
                warn!(%name, "unknown event type");
                Ok(Message::Unknown)
            }
        }
    }
}

/// Data used to initially populate a [Store](crate::store::Store)
#[derive(Debug, Deserialize)]
pub struct InitData {
    // todo: store user segments
    //pub segments: models::user_segments::UserSegments,
    /// Config for all flags
    pub flags: HashMap<String, FeatureFlagState>,
}

/// Update Payload (parsed from json)
#[derive(Debug, Deserialize)]
struct MessagePayload {
    /// updated path
    path: PathBuf,
    /// updated record
    data: Option<serde_json::Value>,
    /// version (used for deletion)
    version: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum FromPatchDataError {
    #[error("Update path is unknown")]
    UnknownPath,

    #[error("Missing flag name")]
    MissingFlagName,

    #[error("Failed to read flag payload")]
    InvalidPayload(#[from] serde_json::Error),
}

/// Used in [Message]
///
/// Describes the change of a single record
/// (add, update or delete)
#[derive(Debug)]
pub enum Update {
    /// a flag changed
    Flag {
        /// name of the flag
        name: String,
        data: Option<FeatureFlagState>,
        version: Option<u64>,
    },
    /// any type of record we haven't implemented
    Unknown,
}

impl TryFrom<MessagePayload> for Update {
    type Error = FromPatchDataError;

    fn try_from(pl: MessagePayload) -> Result<Self, Self::Error> {
        // path iterator
        let mut segments = pl
            .path
            .components()
            .map(|c| c.as_os_str().to_str())
            .flatten()
            .skip_while(|s| *s == "/");

        // first path segment is the type of record
        let first = segments.next().ok_or(FromPatchDataError::UnknownPath)?;
        match first {
            // update for flags
            "flags" => {
                // second path segment is the name
                let name = segments
                    .next()
                    .ok_or(FromPatchDataError::MissingFlagName)?
                    .into();
                let data = pl.data.map(serde_json::from_value).transpose()?;
                Ok(Self::Flag {
                    name,
                    data,
                    version: pl.version,
                })
            }
            // path we don't handle yet
            _ => Ok(Self::Unknown),
        }
    }
}
