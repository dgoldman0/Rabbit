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
                // A simple chat window showing messages and an input
                // composer.  In a production UI this would be bound
                // to the `/q/dialogue` event stream; here it serves
                // as a placeholder demonstrating how a headed
                // burrow might present dialogue.
                markup: r#"<div class='rabbit-dialogue'>
  <h2 class='text-xl font-bold mb-2'>Community Dialogue</h2>
  <div id='messages' class='border p-2 h-48 overflow-y-scroll mb-2'></div>
  <input type='text' placeholder='Say something…' id='composer' class='border p-2 w-full' />
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
  <h2 class='text-xl font-bold mb-2'>Status</h2>
  <p>Connected warrens: <span id='count'></span></p>
</div>"#.into(),
            },
        );
        // Add a control panel route.  This route presents a simple
        // form for listing peers, anchors and trusted burrows, and
        // for connecting or disconnecting to remote peers.  The
        // actual functionality is not implemented in this prototype,
        // but it illustrates how a headed burrow could expose
        // interactive controls over Rabbit itself.
        routes.insert(
            "/control".into(),
            UiRoute {
                path: "/control".into(),
                display_name: "Control Panel".into(),
                ui_hint: "control".into(),
                markup: r#"<div class='rabbit-control'>
  <h2 class='text-xl font-bold mb-2'>Control Panel</h2>
  <p>This panel provides basic controls for discovery and trust.</p>
  <ul class='list-disc ml-4 mb-2'>
    <li><a href='/list/warren' class='text-blue-600 underline'>List peers</a> – shows all known local burrows</li>
    <li><a href='/list/anchors' class='text-blue-600 underline'>List anchors</a> – shows federation anchors</li>
    <li><a href='/list/trusted' class='text-blue-600 underline'>List trusted</a> – shows burrows trusted via TOFU</li>
  </ul>
  <form id='connect-form' class='flex mb-2'>
    <input type='text' placeholder='Burrow ID or address' class='border p-2 flex-grow' />
    <button type='submit' class='bg-blue-500 text-white px-4 py-2 ml-2'>Connect</button>
  </form>
  <form id='disconnect-form' class='flex'>
    <input type='text' placeholder='Burrow ID' class='border p-2 flex-grow' />
    <button type='submit' class='bg-red-500 text-white px-4 py-2 ml-2'>Disconnect</button>
  </form>
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