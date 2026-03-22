# Prototype Community Warren: Willow Glen

This document walks through a small, fictional community warren
implemented using the Rabbit protocol.  The goal is to showcase
hierarchical organisation and cross‑burrow interaction.

## Structure

The root of the warren is named **`willow-glen`**.  It hosts
burrows for governance and local businesses, as well as two
sub‑warrens for families.  Each family sub‑warren contains
individual burrows for parents and children.  A simplified structure
is illustrated below:

```
warren: willow-glen
├── burrow: town-hall
│   ├── /1/agenda
│   ├── /1/regulations
│   └── /q/public-notices
├── burrow: local-market
│   ├── /1/vendors
│   ├── /q/deals
│   └── /u/market-ui
├── warren: oak-family
│   ├── burrow: oak-parent1
│   │   └── /1/photos
│   ├── burrow: oak-parent2
│   │   └── /1/recipes
│   ├── burrow: oak-child1
│   │   └── /1/journal
│   └── burrow: oak-child2
│       └── /1/art
└── warren: pine-family
    ├── burrow: pine-parent
    └── burrow: pine-teen
```

### Menus

Each burrow exposes a **rabbitmap** (similar to a Gopher `gophermap`)
for its root menu.  For example, the `town-hall` burrow’s
`/` menu might look like this:

```
1Agenda      /1/agenda       =
1Regulations /1/regulations  =
qPublic Notices /q/public-notices =
uCivic Portal /u/civic-ui     =
iWelcome to Willow Glen -
.   
```

The leading character on each line denotes the item type:

* `1` – Directory/menu
* `0` – Plain text
* `7` – Search endpoint
* `9` – Binary file
* `q` – Queue/event stream (publish/subscribe)
* `u` – UI bundle or hint
* `i` – Informational/non‑selectable

### Public notices and deals

The `town-hall` and `local-market` burrows both publish live
notifications on their respective queue selectors.  For example,
public notices might include reminders about upcoming meetings or
changes in local regulations, while the market might post daily
discounts and special offers.  Subscribers receive events over a
dedicated **lane** and can reply to them if permitted.

### Family chat

Within the `oak-family` sub‑warren there is a queue called
`/q/chat` used for family chat.  All burrows in the sub‑warren
subscribe to it and can send messages.  This demonstrates how
dialogue emerges naturally from the subscription model without any
special chat primitives.  The `Since` header allows new or
reconnecting burrows to replay missed messages from the continuity
engine.

## Launching the warren

The repository contains a launch harness in
`src/bin/rabbit_launch.rs`.  To spin up the entire community in
process you can run:

```sh
cargo run --bin rabbit_launch --features default
```

This creates three burrows: a headed root (`willow-glen`), and two
headless family nodes.  The burrows automatically discover each
other, establish tunnels, exchange trust information and subscribe
to the appropriate queues.  Feel free to open multiple terminals
and run additional burrows that connect to the warren by specifying
the `--connect` flag.

## Extending the prototype

This warren is intentionally simple.  You can extend it in several
ways:

* Add new burrows for more businesses or services (e.g. a school,
  library, sports league).
* Create additional sub‑warrens for clubs or neighbourhood
  associations.
* Implement search endpoints (`7` items) to allow users to search
  local records.
* Expand the UI bundles to include full HTML templates and
  interactive behaviour.
* Introduce trust manifests for families, linking them to a
  federation anchor.

By experimenting with these extensions you can get a feel for how
Rabbit’s composable primitives—menus, search, publish/subscribe,
identity and trust—can build up complex topologies without a lot
of bespoke infrastructure.
