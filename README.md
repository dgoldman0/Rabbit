# Rabbit Burrow Engine

A **text-based, peer-to-peer, asynchronous protocol engine** for building
federated networks of nodes called *burrows*. Inspired by Gopher's
human-readable simplicity, layered with modern security (Ed25519 + TLS 1.3),
async multiplexing, and native publish/subscribe with replay.

## Quick Start

```bash
# Build everything
cd rabbit_engine
cargo build --release

# Generate a starter config
./target/release/rabbit init

# Start your burrow
./target/release/rabbit serve

# In another terminal, start a second burrow that connects to the first
./target/release/rabbit serve --config config.toml --port 7444 --connect 127.0.0.1:7443

# Or launch a 3-burrow test warren in one command
./target/release/rabbit-warren --count 3 --base-port 7443
```

## CLI Reference

### `rabbit serve`

Start a burrow and listen for incoming connections.

| Flag | Default | Description |
|------|---------|-------------|
| `--config` / `-c` | `config.toml` | Path to config file |
| `--name` | from config | Override burrow display name |
| `--port` / `-p` | from config (7443) | Override listening port |
| `--storage` / `-s` | from config (`data/`) | Override storage directory |
| `--connect` | — | Peer address to connect to on startup (repeatable) |

### `rabbit init`

Generate a starter `config.toml` in the current directory.

| Flag | Default | Description |
|------|---------|-------------|
| `--output` / `-o` | `config.toml` | Output file path |

### `rabbit info`

Show the burrow's identity, port, and content summary.

| Flag | Default | Description |
|------|---------|-------------|
| `--config` / `-c` | `config.toml` | Path to config file |

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
│                CLI / Warren Harness              │  rabbit, rabbit-warren
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
cargo test          # 239 tests
cargo clippy        # 0 warnings
cargo fmt -- --check
```

## Project Structure

```
rabbit_engine/
├── src/
│   ├── bin/
│   │   ├── rabbit.rs           # CLI binary
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
