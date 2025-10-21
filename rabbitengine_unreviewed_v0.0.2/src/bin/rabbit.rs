//! Simple command line interface for launching a single Rabbit burrow.
//!
//! This binary demonstrates how to construct a [`Burrow`](crate::burrow::Burrow)
//! from a configuration and start listening for incoming tunnels or
//! connect to an existing peer.  It uses [`clap`](https://docs.rs/clap)
//! to parse command line arguments.  Because this is a prototype
//! the functionality is deliberately limited: the burrow spawns
//! a listener on the specified port and optionally connects to a
//! single peer.  The UI declaration is chosen based on the
//! `--headed` flag.

use clap::Parser;
use std::path::PathBuf;

use rabbit_warren_impl::{
    burrow::Burrow,
    config::{Config, IdentitySection, NetworkSection, FederationSection},
};

/// Command line options for the `rabbit` binary.
#[derive(Parser, Debug)]
#[command(name = "rabbit", about = "Run a single Rabbit burrow")]
struct Cli {
    /// Human friendly name for this burrow.  Does not have to
    /// match the cryptographic ID.
    #[arg(long, default_value = "burrow")]
    name: String,
    /// Whether to enable a UI declaration (headed mode).  If
    /// false the burrow will run headless.
    #[arg(long, default_value_t = false)]
    headed: bool,
    /// TCP port to listen on for incoming tunnels.
    #[arg(long, default_value_t = 7443)]
    port: u16,
    /// Optional peer address (host:port) to connect to on startup.
    #[arg(long)]
    connect: Option<String>,
    /// Path to a directory where persistent data is stored.
    #[arg(long, default_value = "data")] 
    storage: String,
    /// Path to a directory containing certificates and keys.
    #[arg(long, default_value = "certs")] 
    certs: String,
    /// Path to a PEM file containing trusted root CAs.
    #[arg(long, default_value = "certs/ca.crt")]
    ca: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // Construct a minimal configuration.  In a real application you
    // would read this from a file.  Here we build it from the
    // command line flags.
    let config = Config {
        identity: IdentitySection {
            name: cli.name.clone(),
            storage: cli.storage.clone(),
            certs: cli.certs.clone(),
        },
        network: NetworkSection {
            port: cli.port,
            peers: cli.connect.clone().into_iter().collect(),
        },
        federation: None,
    };
    // Create the burrow.  The `headed` flag selects whether a
    // default UI declaration is loaded.
    let burrow = Burrow::new(config.clone(), cli.headed);
    println!(
        "Starting Rabbit burrow {} (headed={}, port={})",
        burrow.id, cli.headed, cli.port
    );
    // Load any persisted trust state.
    burrow.load_trust().await.ok();
    // Start listening for incoming connections.  Certificates
    // should live in the directory specified by `--certs`.  For
    // simplicity we always use the same file names here.
    let cert_path = format!("{}/burrow.crt", cli.certs);
    let key_path = format!("{}/burrow.key", cli.certs);
    burrow
        .start_listener(&cert_path, &key_path, cli.port)
        .await
        .ok();
    // Optionally connect to a remote peer.
    if let Some(addr) = cli.connect {
        if let Some((host, port_str)) = addr.split_once(':') {
            if let Ok(port) = port_str.parse::<u16>() {
                match burrow.open_tunnel_to_host(host, port, &cli.ca).await {
                    Ok(mut tunnel) => {
                        println!("Connected to peer {}", host);
                        // Perform a basic handshake.
                        let hello = burrow.auth.begin_handshake();
                        tunnel.send_frame(&hello).await.ok();
                    }
                    Err(e) => println!("Failed to connect to {}: {:?}", addr, e),
                }
            }
        }
    }
    // The server runs indefinitely.  Prevent the main task from
    // exiting.  In a real application you would implement proper
    // shutdown handling and signal handling.
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}