# Rabbit Warren Prototype

This repository contains a **prototype implementation** of the
**Rabbit protocol** described in the conversation.  The goal of this
prototype is not to provide a fully‑featured, production‑ready
peer‑to‑peer network, but rather to offer a comprehensive reference
implementation and documentation of the ideas discussed.  In
particular the code demonstrates how one might organise the
components required to build a hierarchical, secure, asynchronous
network of **burrows** (nodes) and **warrens** (collections of nodes).

## Highlights

* **Modular architecture**: the code is organised into small,
  reusable modules.  Each module is extensively commented to help
  orient readers unfamiliar with the domain.
* **Identity and security**: the prototype uses Ed25519 keys for
  identity and binds those keys into self‑signed X.509 certificates
  for transport security.  Trust is established on first use and
  recorded in a local cache.
* **Flexible transport**: the transport layer uses TLS over TCP
  (via the `tokio‑rustls` crate) and supports multiple lanes
  (logical channels) per tunnel.  Messages are encoded in a
  text‑based frame format reminiscent of the original Gopher
  protocol.
* **Discovery and routing**: simple LAN discovery using UDP
  multicast, along with a route table for multi‑hop message
  forwarding.
* **Continuity and persistence**: a default persistence engine
  stores events to disk and can replay them for reconnecting
  subscribers.
* **Delegation and permissions**: a lightweight delegation system
  grants publish/subscribe rights to other burrows on demand.
* **Federation and trust propagation**: anchors can sign trust
  manifests for subordinate burrows, allowing trust to be
  propagated throughout a network.
* **Prototype warren**: the repository includes a test harness and
  documentation for spinning up a small community warren with
  families and businesses.  This demonstrates how the components
  fit together in practice.

## Directory layout

```
└── rabbit_warren_impl
    ├── Cargo.toml      # Rust crate manifest
    ├── README.md       # This file
    ├── docs/           # Additional documentation and examples
    ├── src/            # Source code
    │   ├── bin/        # CLI binaries (e.g. `rabbit` and `rabbit_launch`)
    │   ├── protocol/   # Frame format, lanes, ack, reliability
    │   ├── security/   # Identity, auth, permissions, trust
    │   ├── network/    # Transport, discovery, routing, federation
    │   ├── ui/         # UI declaration and simple HTTP server
    │   ├── burrow/     # The burrow object and tunnel handling
    │   ├── config.rs   # Configuration parsing
    │   └── lib.rs      # Top‑level module re‑exports
    ├── examples/       # Example scenarios
    └── tests/          # Automated integration tests
```

## Running the prototype

The code in this repository is intended as a reference and may
require minor adjustments to build in your environment.  The
`Cargo.toml` file declares all dependencies as optional to avoid
compile time errors when the environment lacks the necessary
prerequisites.  To build the project you can enable the default
feature set:

```sh
cargo build --features default
```

### Launching a test warren

The prototype ships two command line applications in the
`src/bin` directory:

* **`rabbit`** – runs a single burrow.  You can specify the
  burrow’s name, listening port, whether it is headed (with a UI
  declaration) or headless, and an optional peer to connect to.  To
  start two burrows that talk to each other on localhost you might
  run:

  ```sh
  cargo run --bin rabbit -- --name burrow‑a --headed=true --port 7443 &
  cargo run --bin rabbit -- --name burrow‑b --headed=false --port 7444 --connect 127.0.0.1:7443
  ```

  This spins up a headed burrow on port 7443 and a headless burrow on
  port 7444 which connects to the first.  The two burrows establish a
  secure tunnel, exchange UI declarations and subscribe to dialogue
  events.  You can then post messages on the `/dialogue` topic from
  either side and see them appear on the other.

* **`rabbit_launch`** – launches a small warren consisting of a
  headed root burrow and two headless family burrows.  By default
  it binds the root on port 7443 and the families on ports 7444 and
  7445, then connects the families to the root.  To run the example
  warren use:

  ```sh
  cargo run --bin rabbit_launch --features default
  ```

  The harness keeps running until you press `Ctrl‑C`.  You can
  customise the base port and headedness of the root via command
  line flags or by editing `src/bin/rabbit_launch.rs`.

## Prototype community warren

To illustrate the hierarchical nature of Rabbit networks the
repository documents a **Willow Glen community warren**.  This
imaginary warren contains burrows for local governance, businesses
and families.  Each family has its own sub‑warren and each family
member has their own burrow.  The documentation file
[`docs/test_warren.md`](docs/test_warren.md) explains how this
virtual community is structured and provides sample menus and
interactions.  By exploring the test warren you can see how
discovery, routing, federation and dialogue all work together.

## Contributing

This is a toy implementation for educational purposes.  If you
notice errors, have suggestions or would like to improve the
prototype please feel free to open an issue or contribute a patch.
The extensive comments throughout the source are meant to make
navigation and extension straightforward.

## License

This project is distributed under the MIT License.  See
`LICENSE.txt` for details.
