//! `burrow` — run a headless Rabbit burrow node.
//!
//! # Usage
//!
//! ```text
//! burrow serve                     # serve from ./config.toml
//! burrow serve --config path.toml  # serve from a specific config
//! burrow serve --port 8443         # override the listening port
//! burrow init                      # generate a starter config.toml
//! burrow info                      # show burrow identity
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing::{error, info, warn};

use rabbit_engine::burrow::Burrow;
use rabbit_engine::config::Config;
use rabbit_engine::transport::cert::{generate_self_signed, make_server_config, CertPair};
use rabbit_engine::transport::connector::{connect, make_client_config_insecure};
use rabbit_engine::transport::listener::RabbitListener;
use rabbit_engine::ai::connector::spawn_connectors;
use rabbit_engine::ai::http::tls_config;
use rabbit_engine::transport::tunnel::Tunnel;

/// Rabbit burrow — headless peer-to-peer node.
#[derive(Parser)]
#[command(name = "burrow", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start a burrow and listen for connections.
    Serve {
        /// Path to config.toml (default: ./config.toml).
        #[arg(short, long, default_value = "config.toml")]
        config: PathBuf,

        /// Override the burrow's display name.
        #[arg(long)]
        name: Option<String>,

        /// Override the listening port.
        #[arg(short, long)]
        port: Option<u16>,

        /// Override the storage directory.
        #[arg(short, long)]
        storage: Option<PathBuf>,

        /// Connect to a peer on startup (e.g. 127.0.0.1:7444).
        /// Can be specified multiple times.
        #[arg(long)]
        connect: Vec<String>,
    },

    /// Generate a starter config.toml in the current directory.
    Init {
        /// Output path for the config file.
        #[arg(short, long, default_value = "config.toml")]
        output: PathBuf,
    },

    /// Show the burrow's identity (generate one if it doesn't exist).
    Info {
        /// Path to config.toml (default: ./config.toml).
        #[arg(short, long, default_value = "config.toml")]
        config: PathBuf,
    },
}

#[tokio::main]
async fn main() {
    // Initialize tracing (structured logging).
    tracing_subscriber::fmt()
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve {
            config,
            name,
            port,
            storage,
            connect: connect_peers,
        } => {
            if let Err(e) = cmd_serve(config, name, port, storage, connect_peers).await {
                error!("{}", e);
                std::process::exit(1);
            }
        }
        Commands::Init { output } => {
            if let Err(e) = cmd_init(output) {
                error!("{}", e);
                std::process::exit(1);
            }
        }
        Commands::Info { config } => {
            if let Err(e) = cmd_info(config) {
                error!("{}", e);
                std::process::exit(1);
            }
        }
    }
}

// ── Serve ──────────────────────────────────────────────────────

async fn cmd_serve(
    config_path: PathBuf,
    name_override: Option<String>,
    port_override: Option<u16>,
    storage_override: Option<PathBuf>,
    connect_peers: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Load config.
    let mut config = Config::load(&config_path)?;
    let base_dir = config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();

    // Apply CLI overrides.
    if let Some(n) = name_override {
        config.identity.name = n;
    }
    if let Some(p) = port_override {
        config.network.port = p;
    }
    if let Some(s) = storage_override {
        config.identity.storage = s;
    }

    // Merge --connect peers with config peers.
    for peer in &connect_peers {
        if !config.network.peers.contains(peer) {
            config.network.peers.push(peer.clone());
        }
    }

    // Build the burrow.
    let burrow = Arc::new(Burrow::from_config(&config, &base_dir)?);
    info!(
        name = %burrow.name,
        id = %burrow.burrow_id(),
        "burrow identity loaded"
    );

    // Generate or load TLS certificates.
    let cert_dir = base_dir.join(&config.identity.certs);
    let cert_pair = load_or_generate_certs(&cert_dir)?;
    let server_config = make_server_config(&cert_pair)?;

    let listen_addr = format!("0.0.0.0:{}", config.network.port);
    let listener = RabbitListener::bind(&listen_addr, server_config).await?;
    let local_addr = listener.local_addr()?;
    info!(%local_addr, "listening for connections");

    // Spawn outgoing peer connections.
    let client_config = make_client_config_insecure();
    for peer_addr in &config.network.peers {
        let burrow = Arc::clone(&burrow);
        let addr = peer_addr.clone();
        let cc = Arc::clone(&client_config);
        tokio::spawn(async move {
            info!(peer = %addr, "connecting to peer");
            match connect_to_peer(&burrow, &addr, cc).await {
                Ok(id) => info!(peer = %addr, remote_id = %id, "peer session ended"),
                Err(e) => warn!(peer = %addr, err = %e, "peer connection failed"),
            }
        });
    }

    // Spawn AI connectors if configured.
    let _ai_shutdown = if !burrow.ai_chats.is_empty() {
        let ai_tls = tls_config();
        let ai_events = std::sync::Arc::clone(&burrow.events);
        let chats = burrow.ai_chats.clone();
        info!(count = chats.len(), "spawning AI connectors");
        Some(spawn_connectors(chats, ai_events, ai_tls))
    } else {
        None
    };

    // Accept loop — runs until Ctrl-C.
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok(mut tunnel) => {
                        let burrow = Arc::clone(&burrow);
                        tokio::spawn(async move {
                            let peer_addr = "tls-peer";
                            info!(peer = peer_addr, "accepted connection");
                            match burrow.handle_tunnel(&mut tunnel).await {
                                Ok(id) => info!(peer_id = %id, "tunnel closed cleanly"),
                                Err(e) => warn!(err = %e, "tunnel error"),
                            }
                        });
                    }
                    Err(e) => {
                        warn!(err = %e, "accept failed");
                    }
                }
            }
            _ = &mut shutdown => {
                info!("received shutdown signal");
                break;
            }
        }
    }

    // Graceful shutdown: stop AI connectors.
    if let Some(tx) = _ai_shutdown {
        info!("stopping AI connectors");
        let _ = tx.send(true);
    }

    // Save trust cache.
    info!("saving trust cache");
    if let Err(e) = burrow.save_trust() {
        warn!(err = %e, "failed to save trust cache");
    }

    info!("shutdown complete");
    Ok(())
}

/// Connect to a single peer, run client handshake, then dispatch loop.
async fn connect_to_peer(
    burrow: &Burrow,
    addr: &str,
    client_config: Arc<rustls::ClientConfig>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut tunnel = connect(addr, client_config, "localhost").await?;
    let server_id = burrow.client_handshake(&mut tunnel).await?;
    info!(remote_id = %server_id, "handshake complete with peer");

    // Register the peer.
    let peer_info =
        rabbit_engine::warren::peers::PeerInfo::new(server_id.clone(), addr.to_string(), "");
    burrow.peers.register(peer_info).await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    burrow.peers.mark_connected(&server_id, now).await;

    // Run dispatch loop — read frames from the peer.
    let dispatcher = burrow.dispatcher();
    loop {
        let frame = match tunnel.recv_frame().await? {
            Some(f) => f,
            None => break,
        };
        let result = dispatcher.dispatch(&frame, &server_id).await;
        tunnel.send_frame(&result.response).await?;
        for extra in &result.extras {
            tunnel.send_frame(extra).await?;
        }
    }

    burrow.peers.mark_disconnected(&server_id).await;
    Ok(server_id)
}

/// Load TLS certs from disk, or generate and save them.
fn load_or_generate_certs(
    cert_dir: &std::path::Path,
) -> Result<CertPair, Box<dyn std::error::Error>> {
    let cert_path = cert_dir.join("cert.pem");
    let key_path = cert_dir.join("key.pem");

    if cert_path.exists() && key_path.exists() {
        info!("loading TLS certificates from {}", cert_dir.display());
        let cert_pem = std::fs::read_to_string(&cert_path)?;
        let key_pem = std::fs::read_to_string(&key_path)?;
        Ok(CertPair { cert_pem, key_pem })
    } else {
        info!("generating self-signed TLS certificates");
        let pair = generate_self_signed()?;
        std::fs::create_dir_all(cert_dir)?;
        std::fs::write(&cert_path, &pair.cert_pem)?;
        std::fs::write(&key_path, &pair.key_pem)?;
        info!("saved certificates to {}", cert_dir.display());
        Ok(pair)
    }
}

// ── Init ───────────────────────────────────────────────────────

fn cmd_init(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if output.exists() {
        return Err(format!(
            "{} already exists — refusing to overwrite",
            output.display()
        )
        .into());
    }

    let template = r#"# Rabbit burrow configuration
# See SPECS.md for full documentation.

[identity]
name = "my-burrow"
storage = "data/"
certs = "certs/"
require_auth = true

[network]
port = 7443
peers = []

# ── Content ─────────────────────────────────────────────

[[content.menus]]
selector = "/"
items = [
    { type = "i", label = "Welcome to my burrow!" },
    { type = "0", label = "Readme", selector = "/0/readme" },
]

[[content.text]]
selector = "/0/readme"
body = "Hello, world! Edit config.toml to customise this burrow."

# ── Event topics ────────────────────────────────────────

# [[content.topics]]
# path = "/q/chat"
"#;

    std::fs::write(&output, template)?;
    info!("created {}", output.display());
    println!("Created {}", output.display());
    println!("Run `burrow serve` to start your burrow.");
    Ok(())
}

// ── Info ───────────────────────────────────────────────────────

fn cmd_info(config_path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load(&config_path)?;
    let base_dir = config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();

    let burrow = Burrow::from_config(&config, &base_dir)?;

    println!("Burrow: {}", burrow.name);
    println!("ID:     {}", burrow.burrow_id());
    println!("Port:   {}", config.network.port);
    println!(
        "Auth:   {}",
        if config.identity.require_auth {
            "required"
        } else {
            "anonymous"
        }
    );
    println!(
        "Peers:  {}",
        if config.network.peers.is_empty() {
            "(none)".to_string()
        } else {
            config.network.peers.join(", ")
        }
    );

    let menu_count = config.content.menus.len();
    let text_count = config.content.text.len();
    let topic_count = config.content.topics.len();
    println!(
        "Content: {} menus, {} text entries, {} topics",
        menu_count, text_count, topic_count
    );

    Ok(())
}
