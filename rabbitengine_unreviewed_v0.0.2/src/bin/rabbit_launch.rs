//! Launch harness for the Willow Glen test warren.
//!
//! This program spawns a small number of burrows in a single
//! process to simulate a community warren.  It demonstrates how
//! headed and headless burrows can be combined, how tunnels are
//! established and how a simple hierarchical topology can be
//! constructed.  The goal of this binary is not to be a
//! production‑ready orchestrator but rather a concrete example
//! that ties together the various pieces of the Rabbit prototype.
//!
//! The default configuration launches three burrows:
//!
//!  * `willow‑glen` – a headed root burrow that listens on
//!    `base_port`
//!  * `oak‑family` – a headless burrow representing one family
//!  * `pine‑family` – another headless burrow representing a
//!    different family
//!
//! Each headless burrow connects to the root burrow via a
//! `SecureTunnel`.  The burrows then remain running so you can
//! observe their logs.  Feel free to edit this file to add more
//! burrows or change their roles.

use std::sync::Arc;

use clap::Parser;

use rabbit_warren_impl::config::{Config, IdentitySection, NetworkSection, FederationSection};
use rabbit_warren_impl::burrow::Burrow;

/// Command line options for the launch harness.
#[derive(Parser, Debug)]
#[command(author, version, about = "Launch a test warren of burrows", long_about = None)]
struct Options {
    /// Base TCP port for the root burrow.  Family burrows will use
    /// subsequent ports (base+1, base+2, …).
    #[arg(long, default_value_t = 7443)]
    base_port: u16,
    /// Whether to run the root as a headed burrow (UI aware).
    #[arg(long, default_value_t = true)]
    headed_root: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts = Options::parse();

    // Create configuration for the root burrow.  In a complete
    // implementation these values would be read from a TOML file
    // on disk.  Here we construct them inline for clarity.
    let root_config = Config {
        identity: IdentitySection {
            name: "willow‑glen".to_string(),
            storage: "data/willow‑glen".to_string(),
            certs: "certs".to_string(),
        },
        network: NetworkSection {
            port: opts.base_port,
            peers: vec![],
        },
        federation: None,
    };
    // Start the root burrow.
    let root = Arc::new(Burrow::new(root_config.clone(), opts.headed_root));
    root.load_trust().await?;
    // The acceptor is spawned inside start_listener; the path to the
    // certificate and key should point to files generated via
    // `generate_identity_cert` or openssl.  For the purposes of
    // this prototype we reuse a single certificate for all burrows.
    root.start_listener("certs/burrow.crt", "certs/burrow.key", root_config.network.port).await?;
    println!("Started root burrow '{}' on port {}", root_config.identity.name, root_config.network.port);

    // Launch a headless burrow for the oak family.  It will listen on
    // base_port + 1 and connect to the root.
    let oak_config = Config {
        identity: IdentitySection {
            name: "oak‑family".to_string(),
            storage: "data/oak‑family".to_string(),
            certs: "certs".to_string(),
        },
        network: NetworkSection {
            port: opts.base_port + 1,
            peers: vec![format!("127.0.0.1:{}", opts.base_port)],
        },
        federation: None,
    };
    let oak = Arc::new(Burrow::new(oak_config.clone(), false));
    oak.load_trust().await?;
    oak.start_listener("certs/burrow.crt", "certs/burrow.key", oak_config.network.port).await?;
    println!("Started headless burrow '{}' on port {}", oak_config.identity.name, oak_config.network.port);
    // Connect the oak burrow to the root.
    {
        let oak_clone = oak.clone();
        tokio::spawn(async move {
            match oak_clone.open_tunnel_to_host("127.0.0.1", root_config.network.port, "certs/burrow.crt").await {
                Ok(_) => println!("{} connected to root", oak_config.identity.name),
                Err(e) => println!("{} failed to connect to root: {:?}", oak_config.identity.name, e),
            }
        });
    }

    // Launch a headless burrow for the pine family.  It listens on
    // base_port + 2 and connects to the root.
    let pine_config = Config {
        identity: IdentitySection {
            name: "pine‑family".to_string(),
            storage: "data/pine‑family".to_string(),
            certs: "certs".to_string(),
        },
        network: NetworkSection {
            port: opts.base_port + 2,
            peers: vec![format!("127.0.0.1:{}", opts.base_port)],
        },
        federation: None,
    };
    let pine = Arc::new(Burrow::new(pine_config.clone(), false));
    pine.load_trust().await?;
    pine.start_listener("certs/burrow.crt", "certs/burrow.key", pine_config.network.port).await?;
    println!("Started headless burrow '{}' on port {}", pine_config.identity.name, pine_config.network.port);
    {
        let pine_clone = pine.clone();
        tokio::spawn(async move {
            match pine_clone.open_tunnel_to_host("127.0.0.1", root_config.network.port, "certs/burrow.crt").await {
                Ok(_) => println!("{} connected to root", pine_config.identity.name),
                Err(e) => println!("{} failed to connect to root: {:?}", pine_config.identity.name, e),
            }
        });
    }

    // Keep the program alive indefinitely.  In a test you might
    // implement a proper shutdown signal (e.g. ctrl‑c) and call
    // `save_trust` before exiting.
    println!("Test warren launched.  Press Ctrl‑C to exit.");
    futures::future::pending::<()>().await;
    Ok(())
}