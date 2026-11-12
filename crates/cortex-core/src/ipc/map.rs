//! What a daemon answers when asked for the map's address (protocol 12.0 added it): the address
//! the map is served at, with its access token, or why it cannot be served.

use serde::{Deserialize, Serialize};

/// What a daemon answers a request for the map's address with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum MapAnswer {
    /// The map is served.
    Serving {
        /// The address to open, `http://127.0.0.1:<port>/<token>/`. The token in it is the map's
        /// only key: whoever holds the address can read the map.
        url: String,
        /// Whether this build of Cortex holds the map's page and scripts; without them the address
        /// serves a page saying the build was made without the map.
        has_assets: bool,
    },
    /// The map's listener could not be opened on 127.0.0.1.
    BindFailed {
        /// The port asked for; 0 when a free one was asked for.
        port: u16,
        /// What the operating system said.
        cause: String,
    },
}
