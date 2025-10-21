//! Definitions of UI declarations.
//!
//! A UI declaration describes the human facing interface of a
//! headed burrow.  It consists of a version, a description and
//! a set of routes.  Each route includes a path, a display name,
//! a hint about how the content should be rendered and some
//! markup.  This prototype uses basic HTML markup and Tailwind
//! classes; real applications can expand on this.

use std::collections::HashMap;

/// High level declaration of a burrow's user interface.
#[derive(Clone, Debug)]
pub struct UiDeclaration {
    /// Version of the UI declaration format.
    pub version: String,
    /// Human readable description of the UI.
    pub description: String,
    /// Map from route path to route metadata.
    pub routes: HashMap<String, UiRoute>,
}

/// Metadata for a single UI route.
#[derive(Clone, Debug)]
pub struct UiRoute {
    /// Path of the route (e.g. `/dialogue`).
    pub path: String,
    /// Display name shown to the user.
    pub display_name: String,
    /// A hint to the client about how to render the route.
    pub ui_hint: String,
    /// HTML markup associated with the route.
    pub markup: String,
}

impl UiDeclaration {
    /// Return a default UI declaration for a headed burrow.  It
    /// includes two routes: `/dialogue` and `/status`.  The markup
    /// uses simple Tailwind styling for demonstration purposes.
    pub fn default_headed() -> Self {
        let mut routes = HashMap::new();
        routes.insert(
            "/dialogue".into(),
            UiRoute {
                path: "/dialogue".into(),
                display_name: "Community Chat".into(),
                ui_hint: "dialogue".into(),
                markup: r#"<div class='rabbit-dialogue'>
  <h2>Community Dialogue</h2>
  <div id='messages' class='border p-2 h-48 overflow-y-scroll'></div>
  <input type='text' placeholder='Say something…' id='composer' class='border p-2 w-full mt-2' />
</div>"#.into(),
            },
        );
        routes.insert(
            "/status".into(),
            UiRoute {
                path: "/status".into(),
                display_name: "Burrow Status".into(),
                ui_hint: "dashboard".into(),
                markup: r#"<div class='rabbit-status'>
  <h2>Status</h2>
  <p>Connected warrens: <span id='count'></span></p>
</div>"#.into(),
            },
        );
        Self {
            version: "1.0".into(),
            description: "Default headed UI".into(),
            routes,
        }
    }

    /// Return a UI declaration for a headless burrow with no
    /// interactive routes.
    pub fn default_headless() -> Self {
        Self {
            version: "1.0".into(),
            description: "Headless burrow (no UI)".into(),
            routes: HashMap::new(),
        }
    }
}