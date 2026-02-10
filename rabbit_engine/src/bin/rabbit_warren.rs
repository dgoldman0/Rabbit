//! `rabbit-warren` — launch a test warren of N burrows in a single process.
//!
//! Each burrow gets its own TLS listener on `base_port + i`.  The
//! first burrow is the "root"; all others connect to it automatically.
//!
//! # Usage
//!
//! ```text
//! rabbit-warren                           # 3 burrows on ports 7443-7445
//! rabbit-warren --count 5 --base-port 9000
//! rabbit-warren --config-dir ./warrens    # each burrow reads <dir>/burrow-<i>/config.toml
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tracing::{error, info, warn};

use rabbit_engine::burrow::Burrow;
use rabbit_engine::config::Config;
use rabbit_engine::transport::cert::{generate_self_signed, make_server_config};
use rabbit_engine::transport::connector::{connect, make_client_config_insecure};
use rabbit_engine::transport::listener::RabbitListener;
use rabbit_engine::transport::tunnel::Tunnel;

/// Launch a test warren of multiple Rabbit burrows in one process.
#[derive(Parser)]
#[command(name = "rabbit-warren", version, about)]
struct Cli {
    /// Number of burrows to launch.
    #[arg(short = 'n', long, default_value_t = 3)]
    count: usize,

    /// Base port — burrow i listens on base_port + i.
    #[arg(short, long, default_value_t = 7443)]
    base_port: u16,

    /// Directory containing per-burrow config directories.
    /// Each burrow reads `<config-dir>/burrow-<i>/config.toml`.
    /// If absent, default configs are generated in-memory.
    #[arg(long)]
    config_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .init();

    let cli = Cli::parse();

    if cli.count < 2 {
        error!("a warren needs at least 2 burrows");
        std::process::exit(1);
    }

    if let Err(e) = run_warren(cli).await {
        error!("{}", e);
        std::process::exit(1);
    }
}

async fn run_warren(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let cert_pair = generate_self_signed()?;
    let server_config = make_server_config(&cert_pair)?;
    let client_config = make_client_config_insecure();

    // ── Build and start each burrow ────────────────────────────

    struct RunningBurrow {
        burrow: Arc<Burrow>,
        port: u16,
    }

    let mut running: Vec<RunningBurrow> = Vec::new();

    for i in 0..cli.count {
        let port = cli.base_port + i as u16;
        let (config, base_dir) = load_burrow_config(&cli, i, port)?;
        let burrow = Arc::new(Burrow::from_config(&config, &base_dir)?);

        let listen_addr = format!("127.0.0.1:{}", port);
        let listener = RabbitListener::bind(&listen_addr, Arc::clone(&server_config)).await?;
        let actual_port = listener.local_addr()?.port();

        info!(
            index = i,
            name = %burrow.name,
            id = %burrow.burrow_id(),
            port = actual_port,
            "burrow started"
        );

        // Spawn accept loop.
        let burrow_clone = Arc::clone(&burrow);
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok(mut tunnel) => {
                        let b = Arc::clone(&burrow_clone);
                        tokio::spawn(async move {
                            match b.handle_tunnel(&mut tunnel).await {
                                Ok(id) => info!(peer_id = %id, "tunnel closed"),
                                Err(e) => warn!(err = %e, "tunnel error"),
                            }
                        });
                    }
                    Err(e) => {
                        warn!(err = %e, "accept failed");
                        break;
                    }
                }
            }
        });

        running.push(RunningBurrow {
            burrow,
            port: actual_port,
        });
    }

    // ── Connect children to root ───────────────────────────────

    let root_addr = format!("127.0.0.1:{}", running[0].port);

    for (i, rb) in running.iter().enumerate().skip(1) {
        let burrow = Arc::clone(&rb.burrow);
        let addr = root_addr.clone();
        let cc = Arc::clone(&client_config);

        info!(
            child = i,
            child_name = %burrow.name,
            root_addr = %addr,
            "connecting to root"
        );

        // Register the child in the root's peer table so /warren
        // discovery works.
        let child_addr = format!("127.0.0.1:{}", rb.port);
        let mut child_peer = rabbit_engine::warren::peers::PeerInfo::new(
            rb.burrow.burrow_id(),
            &child_addr,
            &rb.burrow.name,
        );
        child_peer.connected = true;
        child_peer.last_seen = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        running[0].burrow.peers.register(child_peer).await;

        let burrow_for_task = Arc::clone(&burrow);
        tokio::spawn(async move {
            match connect_and_dispatch(&burrow_for_task, &addr, cc).await {
                Ok(id) => {
                    info!(child_name = %burrow_for_task.name, remote_id = %id, "peer session ended")
                }
                Err(e) => {
                    warn!(child_name = %burrow_for_task.name, err = %e, "peer connection failed")
                }
            }
        });

        // Small delay so connections don't race.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // ── Print status summary ───────────────────────────────────

    // Give connections a moment to establish.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    println!();
    println!("=== Warren Status ===");
    println!();
    for (i, rb) in running.iter().enumerate() {
        let role = if i == 0 { "root" } else { "child" };
        let peer_count = rb.burrow.peers.count().await;
        println!(
            "  [{}] {} (port {}) — {} — {} peers",
            role,
            rb.burrow.name,
            rb.port,
            rb.burrow.burrow_id(),
            peer_count,
        );
    }
    println!();
    println!("Press Ctrl-C to shut down the warren.");
    println!();

    // Wait for shutdown.
    tokio::signal::ctrl_c().await?;
    info!("shutting down warren");

    for rb in &running {
        if let Err(e) = rb.burrow.save_trust() {
            warn!(name = %rb.burrow.name, err = %e, "failed to save trust cache");
        }
    }

    info!("warren shutdown complete");
    Ok(())
}

/// Connect to a peer and run the dispatch loop.
async fn connect_and_dispatch(
    burrow: &Burrow,
    addr: &str,
    client_config: Arc<rustls::ClientConfig>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut tunnel = connect(addr, client_config, "localhost").await?;
    let server_id = burrow.client_handshake(&mut tunnel).await?;
    info!(remote_id = %server_id, "handshake complete");

    let peer_info =
        rabbit_engine::warren::peers::PeerInfo::new(server_id.clone(), addr.to_string(), "");
    burrow.peers.register(peer_info).await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    burrow.peers.mark_connected(&server_id, now).await;

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

/// Load or generate config for burrow `index`.
fn load_burrow_config(
    cli: &Cli,
    index: usize,
    port: u16,
) -> Result<(Config, PathBuf), Box<dyn std::error::Error>> {
    if let Some(dir) = &cli.config_dir {
        let burrow_dir = dir.join(format!("burrow-{}", index));
        let config_path = burrow_dir.join("config.toml");
        let config = Config::load(&config_path)?;
        Ok((config, burrow_dir))
    } else {
        // Generate an in-memory default config.
        let name = if index == 0 {
            "warren-root".to_string()
        } else {
            format!("burrow-{}", index)
        };

        let toml_str = format!(
            r#"
[identity]
name = "{name}"
require_auth = false

[network]
port = {port}

[[content.menus]]
selector = "/"
items = [
    {{ type = "i", label = "Welcome to {name}" }},
    {{ type = "1", label = "Warren Directory", selector = "/warren" }},
    {{ type = "0", label = "About", selector = "/0/about" }},
]

[[content.text]]
selector = "/0/about"
body = "This is {name}, part of a test warren."
"#
        );

        let config = Config::parse(&toml_str)?;

        // Use a temporary directory for storage.
        let base_dir = std::env::temp_dir().join(format!("rabbit-warren-{}-{}", port, index));
        std::fs::create_dir_all(&base_dir)?;

        Ok((config, base_dir))
    }
}
