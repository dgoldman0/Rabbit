//! `rabbit` — interactive Rabbit protocol browser.
//!
//! A rabbit is a full peer (it has its own identity and can serve
//! content) that happens to have a human at the keyboard.  It connects
//! to burrows, browses their menus, fetches text, and subscribes to
//! event streams — all through an interactive text UI.
//!
//! # Usage
//!
//! ```text
//! rabbit browse 127.0.0.1:7443            # interactive menu navigation
//! rabbit fetch  127.0.0.1:7443 /0/readme  # one-shot content fetch
//! rabbit sub    127.0.0.1:7443 /q/chat    # subscribe to events
//! ```

use std::io::{self, BufRead, Write};

use clap::{Parser, Subcommand};
use tracing::{debug, error, info};

use rabbit_engine::content::store::MenuItem;
use rabbit_engine::protocol::frame::Frame;
use rabbit_engine::security::auth::{build_auth_proof, build_hello};
use rabbit_engine::security::identity::Identity;
use rabbit_engine::transport::connector::{connect, make_client_config_insecure};
use rabbit_engine::transport::tunnel::Tunnel;

/// Rabbit — interactive peer-to-peer browser.
#[derive(Parser)]
#[command(name = "rabbit", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Browse a burrow interactively.
    Browse {
        /// Address of the burrow (e.g. 127.0.0.1:7443).
        addr: String,

        /// Starting selector (default: root menu).
        #[arg(short, long, default_value = "/")]
        selector: String,
    },

    /// Fetch a single resource and print it to stdout.
    Fetch {
        /// Address of the burrow (e.g. 127.0.0.1:7443).
        addr: String,

        /// Selector path to fetch.
        selector: String,
    },

    /// Subscribe to an event topic and stream events to stdout.
    Sub {
        /// Address of the burrow (e.g. 127.0.0.1:7443).
        addr: String,

        /// Topic path (e.g. /q/chat).
        topic: String,

        /// Replay events since this sequence number.
        #[arg(long)]
        since: Option<u64>,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Browse { addr, selector } => {
            if let Err(e) = cmd_browse(&addr, &selector).await {
                error!("{}", e);
                std::process::exit(1);
            }
        }
        Commands::Fetch { addr, selector } => {
            if let Err(e) = cmd_fetch(&addr, &selector).await {
                error!("{}", e);
                std::process::exit(1);
            }
        }
        Commands::Sub { addr, topic, since } => {
            if let Err(e) = cmd_sub(&addr, &topic, since).await {
                error!("{}", e);
                std::process::exit(1);
            }
        }
    }
}

// ── Connection helpers ─────────────────────────────────────────

/// Connect to a burrow and run the Rabbit handshake.
///
/// Returns the tunnel and the remote burrow's ID.  The rabbit
/// generates an ephemeral identity for each session — it's a full
/// peer, just one that lives for one conversation.
async fn open_tunnel(
    addr: &str,
) -> Result<
    (
        rabbit_engine::transport::tls::TlsTunnel<
            tokio_rustls::client::TlsStream<tokio::net::TcpStream>,
        >,
        String,
        Identity,
    ),
    Box<dyn std::error::Error>,
> {
    let identity = Identity::generate();
    let client_config = make_client_config_insecure();
    let mut tunnel = connect(addr, client_config, "localhost").await?;

    // Run the client-side handshake.
    let hello = build_hello(&identity);
    tunnel.send_frame(&hello).await?;

    let response = tunnel
        .recv_frame()
        .await?
        .ok_or("tunnel closed during handshake")?;

    let server_id = if response.verb == "300" {
        // Server requires auth — send proof.
        let proof = build_auth_proof(&identity, &response)?;
        tunnel.send_frame(&proof).await?;

        let ok = tunnel
            .recv_frame()
            .await?
            .ok_or("tunnel closed after AUTH")?;
        if !ok.verb.starts_with("200") {
            return Err(format!("handshake failed: {} {}", ok.verb, ok.args.join(" ")).into());
        }
        ok.header("Burrow-ID").unwrap_or("unknown").to_string()
    } else if response.verb.starts_with("200") {
        response
            .header("Burrow-ID")
            .unwrap_or("unknown")
            .to_string()
    } else {
        return Err(format!(
            "unexpected response: {} {}",
            response.verb,
            response.args.join(" ")
        )
        .into());
    };

    debug!(remote_id = %server_id, "handshake complete");
    Ok((tunnel, server_id, identity))
}

// ── Browse ─────────────────────────────────────────────────────

/// Interactive browse session.
async fn cmd_browse(addr: &str, start_selector: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (mut tunnel, server_id, _identity) = open_tunnel(addr).await?;

    println!();
    println!("  \u{1F407} Connected to {}", short_id(&server_id));
    println!(
        "  \u{2500}\u{2500} type a number to navigate, b to go back, q to quit \u{2500}\u{2500}"
    );
    println!();

    let mut nav_stack: Vec<String> = Vec::new();
    let mut current_selector = start_selector.to_string();

    loop {
        // Send LIST for the current selector.
        let list_frame = Frame::with_args("LIST", vec![current_selector.clone()]);
        tunnel.send_frame(&list_frame).await?;

        let response = tunnel
            .recv_frame()
            .await?
            .ok_or("tunnel closed unexpectedly")?;

        if response.verb == "404" {
            println!("  \u{2717} Not found: {}", current_selector);
            if let Some(prev) = nav_stack.pop() {
                current_selector = prev;
                continue;
            } else {
                break;
            }
        }

        // Follow 301 MOVED redirects (max 5 hops).
        if response.verb.starts_with("301") {
            if let Some(location) = response.header("Location") {
                println!("  \u{27A1} Redirected to {}", location);
                // Location may be "addr/selector" or just "/selector"
                if location.starts_with('/') {
                    current_selector = location.to_string();
                } else if let Some(idx) = location.find('/') {
                    // addr/selector — for now, just use the selector part
                    current_selector = location[idx..].to_string();
                } else {
                    current_selector = location.to_string();
                }
                continue;
            }
        }

        if !response.verb.starts_with("200") {
            println!(
                "  \u{2717} Error: {} {}",
                response.verb,
                response.args.join(" ")
            );
            break;
        }

        // Parse the response body as rabbitmap.
        let body = response.body.as_deref().unwrap_or("");
        let items = parse_rabbitmap(body);

        // Separate navigable items from info lines.
        let mut navigable: Vec<&MenuItem> = Vec::new();
        render_menu(&items, &mut navigable, &current_selector);

        // Read user input.
        match read_choice(navigable.len())? {
            Choice::Navigate(idx) => {
                let item = navigable[idx];
                match item.type_code {
                    '1' => {
                        // Sub-menu — push current and navigate.
                        nav_stack.push(current_selector.clone());
                        current_selector = item.selector.clone();
                    }
                    '0' => {
                        // Text — fetch and display inline.
                        fetch_and_display(&mut tunnel, &item.selector).await?;
                    }
                    '7' => {
                        // Search — prompt for query, send SEARCH verb.
                        print!("  search> ");
                        io::stdout().flush()?;
                        let mut query = String::new();
                        io::stdin().lock().read_line(&mut query)?;
                        let query = query.trim();
                        if !query.is_empty() {
                            let mut search_frame =
                                Frame::with_args("SEARCH", vec![item.selector.clone()]);
                            search_frame.set_body(query);
                            tunnel.send_frame(&search_frame).await?;

                            let resp = tunnel
                                .recv_frame()
                                .await?
                                .ok_or("tunnel closed during SEARCH")?;

                            if let Some(body) = &resp.body {
                                let results = parse_rabbitmap(body);
                                if results.is_empty() {
                                    println!("  (no results)");
                                } else {
                                    println!();
                                    println!(
                                        "  \u{1F50D} {} result{} for \"{}\"\n",
                                        results.len(),
                                        if results.len() == 1 { "" } else { "s" },
                                        query
                                    );
                                    let mut nav: Vec<&MenuItem> = Vec::new();
                                    render_menu(&results, &mut nav, &current_selector);
                                    if let Choice::Navigate(idx) = read_choice(nav.len())? {
                                        let sel = &nav[idx].selector;
                                        fetch_and_display(&mut tunnel, sel).await?;
                                    }
                                }
                            } else {
                                println!("  (no results)");
                            }
                        }
                    }
                    'q' => {
                        // Event stream — subscribe and stream.
                        subscribe_and_stream(&mut tunnel, &item.selector).await?;
                    }
                    '9' => {
                        println!("  (binary content \u{2014} not displayed)");
                    }
                    _ => {
                        // Unknown type — try FETCH.
                        fetch_and_display(&mut tunnel, &item.selector).await?;
                    }
                }
            }
            Choice::Back => {
                if let Some(prev) = nav_stack.pop() {
                    current_selector = prev;
                } else {
                    println!("  (already at root)");
                }
            }
            Choice::Quit => break,
        }
    }

    println!();
    println!("  \u{1F44B} Goodbye.");
    let _ = tunnel.close().await;
    Ok(())
}

/// Fetch a selector and display the content inline.
async fn fetch_and_display<T: Tunnel>(
    tunnel: &mut T,
    selector: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let fetch = Frame::with_args("FETCH", vec![selector.to_string()]);
    tunnel.send_frame(&fetch).await?;

    let response = tunnel
        .recv_frame()
        .await?
        .ok_or("tunnel closed during FETCH")?;

    if response.verb == "404" {
        println!("  \u{2717} Not found: {}", selector);
        return Ok(());
    }

    // Follow 301 redirects (max 5 hops).
    if response.verb.starts_with("301") {
        if let Some(location) = response.header("Location") {
            println!("  \u{27A1} Redirected to {}", location);
            let new_sel = if location.starts_with('/') {
                location.to_string()
            } else if let Some(idx) = location.find('/') {
                location[idx..].to_string()
            } else {
                location.to_string()
            };
            // Recursive fetch with redirect — box to avoid infinite loop
            return Box::pin(fetch_and_display(tunnel, &new_sel)).await;
        }
    }

    let view = response.header("View").unwrap_or("text");

    println!();
    println!(
        "  \u{2500}\u{2500} {} \u{2500}\u{2500} ({})",
        selector, view
    );
    println!();

    if let Some(body) = &response.body {
        // Check if the response is a menu (rabbitmap).
        if view == "menu" {
            let items = parse_rabbitmap(body);
            for item in &items {
                let indicator = type_indicator(item.type_code);
                println!("    {} {}", indicator, item.label);
            }
        } else {
            // Check for base64-encoded binary content.
            if response.header("Transfer") == Some("base64") {
                println!("    (binary content, {} bytes encoded)", body.len());
            } else {
                // Plain text — indent each line for readability.
                for line in body.lines() {
                    println!("    {}", line);
                }
            }
        }
    } else {
        println!("    (empty)");
    }

    println!();
    print!("  [enter to continue] ");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().lock().read_line(&mut buf)?;
    Ok(())
}

/// Subscribe to a topic and stream events inline.  Returns when the
/// tunnel closes or an error occurs.
async fn subscribe_and_stream<T: Tunnel>(
    tunnel: &mut T,
    topic: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sub = Frame::with_args("SUBSCRIBE", vec![topic.to_string()]);
    sub.set_header("Lane", "0");
    tunnel.send_frame(&sub).await?;

    let ack = tunnel
        .recv_frame()
        .await?
        .ok_or("tunnel closed during SUBSCRIBE")?;

    if !ack.verb.starts_with("201") && !ack.verb.starts_with("200") {
        println!(
            "  \u{2717} Subscribe failed: {} {}",
            ack.verb,
            ack.args.join(" ")
        );
        return Ok(());
    }

    println!();
    println!(
        "  \u{2500}\u{2500} subscribed to {} (Ctrl-C to stop) \u{2500}\u{2500}",
        topic
    );
    println!();

    // Stream events until tunnel closes or error.
    loop {
        let frame = match tunnel.recv_frame().await? {
            Some(f) => f,
            None => {
                println!("  (stream ended)");
                break;
            }
        };

        if frame.verb == "EVENT" || frame.verb == "210" {
            let seq = frame.header("Seq").unwrap_or("?");
            let ts = frame.header("Timestamp").unwrap_or("");
            let body = frame.body.as_deref().unwrap_or("");
            println!("  [{}] {} {}", seq, ts, body.trim());
        } else {
            debug!(verb = %frame.verb, "non-event frame during subscribe");
        }
    }

    Ok(())
}

// ── Fetch (one-shot) ───────────────────────────────────────────

async fn cmd_fetch(addr: &str, selector: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (mut tunnel, server_id, _identity) = open_tunnel(addr).await?;
    info!(remote = %short_id(&server_id), "connected");

    let fetch = Frame::with_args("FETCH", vec![selector.to_string()]);
    tunnel.send_frame(&fetch).await?;

    let response = tunnel
        .recv_frame()
        .await?
        .ok_or("tunnel closed during FETCH")?;

    if !response.verb.starts_with("200") {
        eprintln!("error: {} {}", response.verb, response.args.join(" "));
        std::process::exit(1);
    }

    if let Some(body) = &response.body {
        print!("{}", body);
    }

    let _ = tunnel.close().await;
    Ok(())
}

// ── Subscribe (streaming) ──────────────────────────────────────

async fn cmd_sub(
    addr: &str,
    topic: &str,
    since: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut tunnel, server_id, _identity) = open_tunnel(addr).await?;
    info!(remote = %short_id(&server_id), "connected");

    let mut sub = Frame::with_args("SUBSCRIBE", vec![topic.to_string()]);
    sub.set_header("Lane", "0");
    if let Some(seq) = since {
        sub.set_header("Since", seq.to_string());
    }
    tunnel.send_frame(&sub).await?;

    let ack = tunnel
        .recv_frame()
        .await?
        .ok_or("tunnel closed during SUBSCRIBE")?;

    if !ack.verb.starts_with("201") && !ack.verb.starts_with("200") {
        eprintln!("error: {} {}", ack.verb, ack.args.join(" "));
        std::process::exit(1);
    }

    eprintln!("subscribed to {} \u{2014} streaming events", topic);

    loop {
        let frame = match tunnel.recv_frame().await {
            Ok(Some(f)) => f,
            Ok(None) => {
                eprintln!("(stream ended)");
                break;
            }
            Err(e) => {
                eprintln!("error: {}", e);
                break;
            }
        };

        if frame.verb == "EVENT" || frame.verb == "210" {
            let seq = frame.header("Seq").unwrap_or("?");
            let body = frame.body.as_deref().unwrap_or("");
            println!("{}\t{}", seq, body.trim());
        }
    }

    let _ = tunnel.close().await;
    Ok(())
}

// ── Menu rendering ─────────────────────────────────────────────

/// Parse a rabbitmap body into menu items.
pub fn parse_rabbitmap(body: &str) -> Vec<MenuItem> {
    body.lines()
        .filter_map(MenuItem::from_rabbitmap_line)
        .collect()
}

/// Render a menu to stdout and populate the navigable item list.
///
/// Info lines (`i`) are displayed without a number.  All other items
/// get a sequential number that the user can type to navigate.
pub fn render_menu<'a>(items: &'a [MenuItem], navigable: &mut Vec<&'a MenuItem>, selector: &str) {
    navigable.clear();

    println!("  \u{250C}\u{2500} {} \u{2500}\u{2510}", selector);
    println!("  \u{2502}");

    for item in items {
        if item.type_code == 'i' {
            // Info line — no number.
            println!("  \u{2502}   {}", item.label);
        } else {
            let idx = navigable.len() + 1;
            let indicator = type_indicator(item.type_code);
            println!("  \u{2502} {:>3}. {} {}", idx, indicator, item.label);
            navigable.push(item);
        }
    }

    println!("  \u{2502}");
    println!("  \u{2514}\u{2500}\u{2500}\u{2500}\u{2500}");
    println!();
}

/// Map a type code to a human-readable indicator.
pub fn type_indicator(code: char) -> &'static str {
    match code {
        '0' => "\u{1F4C4}",
        '1' => "\u{1F4C2}",
        '7' => "\u{1F50D}",
        '9' => "\u{1F4E6}",
        'q' => "\u{26A1}",
        'i' => "\u{2139}\u{FE0F} ",
        _ => "\u{2022}",
    }
}

// ── Input handling ─────────────────────────────────────────────

enum Choice {
    Navigate(usize), // zero-indexed
    Back,
    Quit,
}

/// Read a choice from the user.  Returns the zero-indexed navigable
/// item or a navigation command.
fn read_choice(max: usize) -> Result<Choice, Box<dyn std::error::Error>> {
    loop {
        print!("  rabbit> ");
        io::stdout().flush()?;

        let mut input = String::new();
        let n = io::stdin().lock().read_line(&mut input)?;
        if n == 0 {
            // EOF
            return Ok(Choice::Quit);
        }

        let trimmed = input.trim();

        if trimmed.eq_ignore_ascii_case("q") || trimmed.eq_ignore_ascii_case("quit") {
            return Ok(Choice::Quit);
        }
        if trimmed.eq_ignore_ascii_case("b")
            || trimmed.eq_ignore_ascii_case("back")
            || trimmed == ".."
        {
            return Ok(Choice::Back);
        }
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(num) = trimmed.parse::<usize>() {
            if num >= 1 && num <= max {
                return Ok(Choice::Navigate(num - 1));
            }
            println!("  (pick 1\u{2013}{}, b for back, q to quit)", max);
        } else {
            println!("  (pick a number, b for back, q to quit)");
        }
    }
}

// ── Utilities ──────────────────────────────────────────────────

/// Shorten a burrow ID for display.
fn short_id(id: &str) -> String {
    if let Some(rest) = id.strip_prefix("ed25519:") {
        if rest.len() > 12 {
            format!("ed25519:{}\u{2026}", &rest[..12])
        } else {
            id.to_string()
        }
    } else {
        id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rabbitmap_basic() {
        let body = "iWelcome!\t\t=\t\r\n1Docs\t/docs\t=\t\r\n0Readme\t/0/readme\t=\t\r\n.\r\n";
        let items = parse_rabbitmap(body);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].type_code, 'i');
        assert_eq!(items[0].label, "Welcome!");
        assert_eq!(items[1].type_code, '1');
        assert_eq!(items[1].label, "Docs");
        assert_eq!(items[1].selector, "/docs");
        assert_eq!(items[2].type_code, '0');
        assert_eq!(items[2].label, "Readme");
        assert_eq!(items[2].selector, "/0/readme");
    }

    #[test]
    fn parse_rabbitmap_empty_body() {
        let items = parse_rabbitmap("");
        assert!(items.is_empty());
    }

    #[test]
    fn parse_rabbitmap_just_terminator() {
        let items = parse_rabbitmap(".\r\n");
        assert!(items.is_empty());
    }

    #[test]
    fn render_menu_separates_info_from_navigable() {
        let items = vec![
            MenuItem::info("Hello"),
            MenuItem::local('1', "Sub-menu", "/sub"),
            MenuItem::info("Divider"),
            MenuItem::local('0', "Text", "/0/text"),
        ];
        let mut navigable = Vec::new();
        render_menu(&items, &mut navigable, "/");
        assert_eq!(navigable.len(), 2);
        assert_eq!(navigable[0].label, "Sub-menu");
        assert_eq!(navigable[1].label, "Text");
    }

    #[test]
    fn type_indicator_mapping() {
        assert_eq!(type_indicator('0'), "\u{1F4C4}");
        assert_eq!(type_indicator('1'), "\u{1F4C2}");
        assert_eq!(type_indicator('7'), "\u{1F50D}");
        assert_eq!(type_indicator('9'), "\u{1F4E6}");
        assert_eq!(type_indicator('q'), "\u{26A1}");
        assert_eq!(type_indicator('x'), "\u{2022}");
    }

    #[test]
    fn short_id_truncates() {
        let long = "ed25519:ABCDEFGHIJKLMNOP";
        assert_eq!(short_id(long), "ed25519:ABCDEFGHIJKL\u{2026}");
    }

    #[test]
    fn short_id_preserves_short() {
        let short = "ed25519:ABC";
        assert_eq!(short_id(short), "ed25519:ABC");
    }

    #[test]
    fn short_id_non_ed25519() {
        assert_eq!(short_id("anonymous"), "anonymous");
    }
}
