# rabbit-cli — Python Client for the Rabbit Protocol

A pure-Python command-line client for navigating Rabbit warrens.
Implements the core Rabbit wire protocol: TLS 1.3 transport,
Ed25519 identity, frame parsing, menu browsing, content fetching,
event subscription, and search.

## Install

```bash
pip install -e .
```

Or just install dependencies and run directly:

```bash
pip install cryptography
python -m rabbit_client browse 127.0.0.1:7443
```

## Usage

### Interactive Browse

```bash
rabbit browse 127.0.0.1:7443
rabbit browse 127.0.0.1:7443 --selector /1/docs
```

Interactive commands:
- **number** — navigate to a menu item
- **b** / **back** — go back
- **/search term** — search from current context
- **q** / **quit** — exit

### Fetch a Resource

```bash
rabbit fetch 127.0.0.1:7443 /0/readme
```

### List a Directory

```bash
rabbit list 127.0.0.1:7443 /
rabbit list 127.0.0.1:7443 /1/docs
```

### Subscribe to Events

```bash
rabbit sub 127.0.0.1:7443 /q/chat
rabbit sub 127.0.0.1:7443 /q/chat --since 5
```

### Describe a Resource

```bash
rabbit describe 127.0.0.1:7443 /0/readme
```

### Publish to a Topic

```bash
rabbit pub 127.0.0.1:7443 /q/chat "Hello from Python!"
```

## Architecture

```
rabbit_client/
├── __init__.py       # Package marker
├── __main__.py       # python -m rabbit_client entry point
├── protocol.py       # Frame serialization/parsing, constants
├── transport.py      # TLS connection, async-free frame I/O
├── identity.py       # Ed25519 keypair, signing, Burrow-ID
├── session.py        # Handshake (HELLO/CHALLENGE/AUTH)
├── menu.py           # Rabbitmap parsing, MenuItem
├── browser.py        # Interactive navigation loop
└── cli.py            # argparse CLI dispatcher
```

Pure Python. Single dependency: `cryptography` (for Ed25519).
No async runtime — uses plain sockets with `ssl` for TLS.

## Protocol Compatibility

Implements Rabbit/1.0 wire protocol as specified in SPECS.md:
- UTF-8 text frames with CRLF line endings
- `End:\r\n` header terminator
- Tab-delimited rabbitmap menus with `.` terminator
- Ed25519 challenge/response authentication
- TLS 1.3 with ALPN `rabbit/1`
- Lane 0 control, Txn correlation
- Credit-based flow control (client-side)
