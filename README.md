# Rabbit Burrow Engine

A **text-based, peer-to-peer, asynchronous protocol engine** for building
federated networks of nodes called *burrows*. Inspired by Gopher's
human-readable simplicity, layered with modern security (Ed25519 + TLS 1.3),
async multiplexing, and native publish/subscribe with replay.

Three binaries, one crate:

| Binary | Role |
|--------|------|
| `burrow` | Headless server node — serves content, routes messages, runs unattended |
| `rabbit` | Interactive browser — a full peer with a human at the keyboard |
| `rabbit-warren` | Multi-burrow test harness — launches several nodes in one process |

## Quick Start

```bash
# Build everything
cd rabbit_engine
cargo build --release

# Generate a starter config and start a headless burrow
./target/release/burrow init
./target/release/burrow serve

# In another terminal, browse it interactively
./target/release/rabbit browse 127.0.0.1:7443

# Or fetch a specific resource to stdout
./target/release/rabbit fetch 127.0.0.1:7443 /0/readme

# Subscribe to an event stream
./target/release/rabbit sub 127.0.0.1:7443 /q/chat

# Launch a multi-burrow test warren
./target/release/rabbit-warren --count 3 --base-port 7443
```

## CLI Reference

### `burrow serve`

Start a headless burrow node and listen for connections.

| Flag | Default | Description |
|------|---------|-------------|
| `--config` / `-c` | `config.toml` | Path to config file |
| `--name` | from config | Override burrow display name |
| `--port` / `-p` | from config (7443) | Override listening port |
| `--storage` / `-s` | from config (`data/`) | Override storage directory |
| `--connect` | — | Peer address to connect to on startup (repeatable) |

### `burrow init`

Generate a starter `config.toml` in the current directory.

| Flag | Default | Description |
|------|---------|-------------|
| `--output` / `-o` | `config.toml` | Output file path |

### `burrow info`

Show the burrow's identity, port, and content summary.

| Flag | Default | Description |
|------|---------|-------------|
| `--config` / `-c` | `config.toml` | Path to config file |

### `rabbit browse`

Browse a burrow interactively. Connects via TLS, runs a full
handshake (the rabbit is a peer with its own ephemeral identity),
then displays menus in a text UI with numbered navigation.

| Arg / Flag | Default | Description |
|------------|---------|-------------|
| `<addr>` | (required) | Burrow address (e.g. `127.0.0.1:7443`) |
| `--selector` / `-s` | `/` | Starting menu selector |

Interactive commands: **number** to navigate, **b** to go back, **q** to quit.

### `rabbit fetch`

Fetch a single resource and print its body to stdout.

| Arg | Description |
|-----|-------------|
| `<addr>` | Burrow address |
| `<selector>` | Resource path (e.g. `/0/readme`) |

### `rabbit sub`

Subscribe to an event topic and stream events to stdout.

| Arg / Flag | Description |
|------------|-------------|
| `<addr>` | Burrow address |
| `<topic>` | Topic path (e.g. `/q/chat`) |
| `--since` | Replay events since sequence number |

### `rabbit-warren`

Launch a multi-burrow test warren in a single process.

| Flag | Default | Description |
|------|---------|-------------|
| `--count` / `-n` | 3 | Number of burrows |
| `--base-port` / `-b` | 7443 | First burrow's port (subsequent use port+1, port+2, …) |
| `--config-dir` | — | Directory with per-burrow configs (`burrow-0/`, `burrow-1/`, …) |

## Example `config.toml`

```toml
[identity]
name = "my-burrow"
storage = "data/"
certs = "certs/"
require_auth = true

[network]
port = 7443
peers = ["192.168.1.10:7443"]

[[content.menus]]
selector = "/"
items = [
    { type = "i", label = "Welcome to my burrow!" },
    { type = "0", label = "Readme", selector = "/0/readme" },
    { type = "1", label = "Documents", selector = "/1/docs" },
    { type = "q", label = "Chat", selector = "/q/chat" },
]

[[content.text]]
selector = "/0/readme"
body = "Hello! This is my burrow."

[[content.text]]
selector = "/0/guide"
file = "content/guide.txt"

[[content.topics]]
path = "/q/chat"
```

## Architecture

```
┌─────────────────────────────────────────────────┐
│          CLI / Browser / Warren Harness           │  burrow, rabbit, rabbit-warren
├─────────────────────────────────────────────────┤
│                     Burrow                       │  Ties everything together
├──────────┬───────────┬───────────┬──────────────┤
│ Dispatch │  Content  │   Events  │   Discovery  │  Routing, menus, pub/sub
├──────────┴───────────┴───────────┴──────────────┤
│                   Security                       │  Identity, auth, trust, caps
├─────────────────────────────────────────────────┤
│                   Protocol                       │  Frame, lane, txn, flow ctrl
├─────────────────────────────────────────────────┤
│                   Transport                      │  TLS tunnels + memory test
└─────────────────────────────────────────────────┘
```

Each layer depends only on the layers below it. No circular dependencies.

### Key Concepts

| Term | Description |
|------|-------------|
| **Burrow** | A node identified by an Ed25519 keypair. Serves content, routes messages, manages subscriptions. |
| **Warren** | A connected group of burrows. Warrens nest recursively. |
| **Tunnel** | A TLS 1.3 connection between two burrows. Full-duplex, persistent. |
| **Frame** | The atomic protocol unit — UTF-8 text with CRLF line endings, `End:` terminator, optional body. |
| **Selector** | A path referencing a resource (e.g., `/0/readme`, `/q/chat`). |
| **Lane** | A logical async channel within a tunnel with independent flow control. |

### Wire Protocol

All frames are human-readable UTF-8:

```
LIST /\r\n
Lane: L0\r\n
Txn: T-1\r\n
End:\r\n
```

Response:

```
200 MENU\r\n
Txn: T-1\r\n
Length: 42\r\n
End:\r\n
i	Welcome to my burrow!	=	=
0	Readme	/0/readme	=
.
```

No JSON. No binary serialization. A human can read the traffic on the wire.

## Dependencies

| Crate | Purpose |
|-------|---------|
| `tokio` | Async runtime, networking, sync primitives |
| `thiserror` | Error type derivation |
| `ed25519-dalek` | Ed25519 identity, signing, verification |
| `rand` | Secure random (nonces, tokens) |
| `sha2` | SHA-256 fingerprints |
| `base32` | Burrow ID encoding |
| `rustls` + `tokio-rustls` | TLS 1.3 transport |
| `rustls-pemfile` | PEM certificate loading |
| `rcgen` | Self-signed certificate generation |
| `serde` + `toml` | TOML config parsing (config only — never wire protocol) |
| `clap` | CLI argument parsing |
| `tracing` + `tracing-subscriber` | Structured logging |

## Testing

```bash
cargo test          # 247 tests
cargo clippy        # 0 warnings
cargo fmt -- --check
```

## Project Structure

```
rabbit_engine/
├── src/
│   ├── bin/
│   │   ├── burrow.rs           # Headless server node
│   │   ├── rabbit.rs           # Interactive browser
│   │   └── rabbit_warren.rs    # Warren launcher
│   ├── burrow.rs               # Top-level assembly
│   ├── config.rs               # TOML config
│   ├── protocol/               # Frame, lane, txn, errors
│   ├── security/               # Identity, auth, trust, caps
│   ├── transport/              # TLS + memory tunnels
│   ├── dispatch/               # Frame routing
│   ├── content/                # Menus, text, loader
│   ├── events/                 # Pub/sub, continuity
│   ├── warren/                 # Peer table, discovery
│   └── lib.rs
└── tests/                      # Integration tests
```

## Documentation

- [SPECS.md](SPECS.md) — Full MVP protocol specification
- [PLAN.md](PLAN.md) — 6-phase implementation roadmap
- [PROGRESSREPORT.md](PROGRESSREPORT.md) — Detailed per-phase progress

## License

MIT
