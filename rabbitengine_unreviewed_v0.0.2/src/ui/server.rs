//! Simple HTTP server for Rabbit UI.
//!
//! This module uses [`warp`](https://crates.io/crates/warp) to
//! serve the markup specified in a [`UiDeclaration`](crate::ui::declaration::UiDeclaration).
//! Each route defined in the declaration becomes an HTTP endpoint
//! under the same path.  For example if the declaration includes
//! a route `/dialogue` then a GET request to `/dialogue` on the
//! burrow's host will return the associated HTML.

use anyhow::Result;
use warp::{Filter, Reply, http::Response};
use std::sync::Arc;

use super::declaration::UiDeclaration;

/// Start an HTTP server to serve the UI.  It listens on the
/// specified port on localhost.  The server runs indefinitely
/// until the process is terminated.  Callers should spawn this
/// function on its own task if other work needs to run
/// concurrently.
#[cfg(feature = "ui")]
pub async fn start_ui(ui: Arc<UiDeclaration>, port: u16) -> Result<()> {
    let mut route_filters = Vec::new();
    for (path, route) in ui.routes.clone() {
        let markup = route.markup.clone();
        let route_path = path.trim_start_matches('/').to_string();
        let filter = warp::path(route_path)
            .and(warp::get())
            .map(move || Response::builder()
                .header("content-type", "text/html; charset=utf-8")
                .body(markup.clone()));
        route_filters.push(filter.boxed());
    }
    let routes = if route_filters.is_empty() {
        warp::any()
            .map(|| Response::builder().status(404).body("Not Found"))
            .boxed()
    } else {
        let mut iter = route_filters.into_iter();
        let mut combined = iter.next().unwrap();
        for r in iter {
            combined = combined.or(r).boxed();
        }
        combined
    };
    warp::serve(routes).run(([127, 0, 0, 1], port)).await;
    Ok(())
}

/// Stub when the `ui` feature is disabled.
#[cfg(not(feature = "ui"))]
pub async fn start_ui(_ui: Arc<UiDeclaration>, _port: u16) -> Result<()> {
    Ok(())
}