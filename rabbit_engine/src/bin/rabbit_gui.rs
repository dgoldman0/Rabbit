//! `rabbit-gui` — Graphical Rabbit protocol browser.
//!
//! Connects to a burrow, fetches content, renders it as HTML using
//! the AI view generator (or fallback), and displays it in a
//! Dioxus/WebView desktop window.
//!
//! Build: `cargo build --features gui --bin rabbit-gui`
//! Run:   `cargo run  --features gui --bin rabbit-gui -- <host:port> [selector]`

use clap::Parser;

use rabbit_engine::config::Config;
use rabbit_engine::gui::theme::Theme;
use rabbit_engine::gui::view_gen::{fallback_html, ViewContent};

#[cfg(feature = "gui")]
use rabbit_engine::gui::app::launch_gui;

/// Graphical Rabbit protocol browser.
#[derive(Parser, Debug)]
#[command(name = "rabbit-gui", about = "Graphical Rabbit protocol browser")]
struct Args {
    /// Burrow host to connect to (host:port).
    host: String,

    /// Initial selector to fetch (default: root menu "/").
    #[arg(default_value = "/")]
    selector: String,

    /// Path to config file.
    #[arg(short, long, default_value = "rabbit.toml")]
    config: String,
}

fn main() {
    let args = Args::parse();

    // Load config (or use defaults).
    let config = Config::load(&args.config).unwrap_or_default();
    let gui_config = config.gui.clone();
    let _theme = Theme::parse(&gui_config.theme);

    // Generate initial HTML (fallback until we connect and fetch).
    let _initial_html = fallback_html(
        &ViewContent::Loading {
            selector: args.selector.clone(),
        },
        &gui_config.theme,
    );

    #[cfg(feature = "gui")]
    {
        eprintln!(
            "rabbit-gui: connecting to {} selector={}",
            args.host, args.selector
        );
        launch_gui(gui_config, _initial_html);
    }

    #[cfg(not(feature = "gui"))]
    {
        eprintln!(
            "rabbit-gui: the 'gui' feature is not enabled.\n\
             Rebuild with: cargo build --features gui --bin rabbit-gui"
        );
        std::process::exit(1);
    }
}
