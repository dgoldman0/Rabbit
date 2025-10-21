//! Configuration parser for Rabbit burrows.
//!
//! The [`Config`] structure represents the contents of a typical
//! `config.toml` file used to configure an instance of a burrow.
//! Configuration is optional—if no configuration is supplied the
//! code falls back to sensible defaults.  The structure is
//! serialisable and deserialisable via `serde` so you can read and
//! write TOML easily.
//!
//! Example `config.toml`:
//!
//! ```toml
//! [identity]
//! name = "oak-parent"
//! storage = "data/"
//! certs = "certs/"
//!
//! [network]
//! port = 7443
//! peers = ["127.0.0.1:7444"]
//!
//! [federation]
//! anchors = ["oak-federation"]
//! ```
//!
//! See the README for more details on each section.

use serde::Deserialize;
use std::fs;
use std::path::Path;
use anyhow::{anyhow, Result};

/// Top‑level configuration structure.  Each section corresponds
/// to a table in the TOML file.
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// Identity and storage parameters.
    pub identity: IdentitySection,
    /// Network parameters (ports, peers).
    pub network: NetworkSection,
    /// Optional federation parameters.  When present, the burrow
    /// participates in a federation and looks up anchors by ID.
    pub federation: Option<FederationSection>,
}

/// Identity configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct IdentitySection {
    /// A human‑friendly name for the burrow.  Does not have to
    /// match the cryptographic ID but can be used in menus and UI.
    pub name: String,
    /// Path to a directory where persistent state (e.g. continuity
    /// logs, trust cache) should be stored.
    pub storage: String,
    /// Path to a directory where certificates and keys should be
    /// generated and loaded from.
    pub certs: String,
}

/// Network configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct NetworkSection {
    /// The TCP port on which the burrow listens for incoming
    /// connections.  If multiple burrows run on the same machine
    /// each should be assigned a unique port.
    pub port: u16,
    /// A list of peer addresses (host:port) to which the burrow
    /// should attempt to connect on startup.  Use this to join an
    /// existing warren.
    pub peers: Vec<String>,
}

/// Optional federation configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct FederationSection {
    /// A list of IDs of anchors that the burrow trusts.  These
    /// correspond to other warrens that can sign trust manifests.
    pub anchors: Vec<String>,
}

impl Config {
    /// Load configuration from a file.  If the file does not
    /// exist an error is returned.  See the top of this file for
    /// an example configuration.
    pub fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .map_err(|e| anyhow!("failed to read config file: {}", e))?;
        let cfg: Self = toml::from_str(&contents)
            .map_err(|e| anyhow!("failed to parse config: {}", e))?;
        Ok(cfg)
    }
}
