"""Command-line interface for the Rabbit client.

Subcommands:
    browse   Interactive menu navigation
    fetch    One-shot content retrieval
    list     List a directory / menu
    sub      Subscribe to an event stream
    pub      Publish a message to a topic
    describe Show metadata for a selector
"""

from __future__ import annotations

import argparse
import sys

from .browser import Browser, _C, _err, _header, _info, _ok, _selector
from .menu import parse_menu
from .protocol import ProtocolError
from .session import Session


def _parse_addr(addr: str) -> tuple[str, int]:
    """Parse ``host:port`` with default port 7443."""
    if ":" in addr:
        host, port_s = addr.rsplit(":", 1)
        return host, int(port_s)
    return addr, 7443


# -- Subcommands ---------------------------------------------------------

def cmd_browse(args: argparse.Namespace) -> None:
    """Interactive browse."""
    host, port = _parse_addr(args.addr)
    try:
        with Session(host, port) as sess:
            browser = Browser(sess)
            browser.run(start_selector=args.selector)
    except ProtocolError as e:
        print(_err(f"Protocol error: {e}"), file=sys.stderr)
        sys.exit(1)
    except ConnectionRefusedError:
        print(_err(f"Connection refused: {args.addr}"), file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        print()


def cmd_fetch(args: argparse.Namespace) -> None:
    """Fetch a resource and print to stdout."""
    host, port = _parse_addr(args.addr)
    try:
        with Session(host, port) as sess:
            resp = sess.fetch(args.selector)
            if resp.is_error:
                print(f"Error {resp.verb}: {resp.body.strip()}", file=sys.stderr)
                sys.exit(1)
            print(resp.body, end="")
    except ProtocolError as e:
        print(f"Protocol error: {e}", file=sys.stderr)
        sys.exit(1)
    except ConnectionRefusedError:
        print(f"Connection refused: {args.addr}", file=sys.stderr)
        sys.exit(1)


def cmd_list(args: argparse.Namespace) -> None:
    """List a menu and print items."""
    host, port = _parse_addr(args.addr)
    try:
        with Session(host, port) as sess:
            resp = sess.list(args.selector)
            if resp.is_error:
                print(f"Error {resp.verb}: {resp.body.strip()}", file=sys.stderr)
                sys.exit(1)
            items = parse_menu(resp.body)
            for item in items:
                if item.is_info:
                    print(f"  {item.glyph}  {item.label}")
                else:
                    loc = f" → {item.burrow}" if item.is_remote else ""
                    print(f"  {item.glyph}  {item.label}  {_C.DIM}[{item.selector}]{_C.RESET}{loc}")
    except ProtocolError as e:
        print(f"Protocol error: {e}", file=sys.stderr)
        sys.exit(1)
    except ConnectionRefusedError:
        print(f"Connection refused: {args.addr}", file=sys.stderr)
        sys.exit(1)


def cmd_sub(args: argparse.Namespace) -> None:
    """Subscribe to an event stream."""
    host, port = _parse_addr(args.addr)
    since = str(args.since) if args.since else ""
    try:
        with Session(host, port) as sess:
            resp = sess.subscribe(args.topic, since=since)
            if resp.is_error:
                print(f"Error {resp.verb}: {resp.body.strip()}", file=sys.stderr)
                sys.exit(1)
            # Stream events to stdout
            while True:
                ev = sess.recv_event(timeout=120.0)
                if ev is None:
                    break
                if ev.is_event:
                    seq = ev.get("Seq", "?")
                    body = ev.body.rstrip()
                    print(f"{seq}\t{body}", flush=True)
    except ProtocolError as e:
        print(f"Protocol error: {e}", file=sys.stderr)
        sys.exit(1)
    except ConnectionRefusedError:
        print(f"Connection refused: {args.addr}", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        print()


def cmd_pub(args: argparse.Namespace) -> None:
    """Publish a message to a topic."""
    host, port = _parse_addr(args.addr)
    try:
        with Session(host, port) as sess:
            resp = sess.publish(args.topic, args.message)
            if resp.is_error:
                print(f"Error {resp.verb}: {resp.body.strip()}", file=sys.stderr)
                sys.exit(1)
            print(_ok("Published."))
    except ProtocolError as e:
        print(f"Protocol error: {e}", file=sys.stderr)
        sys.exit(1)
    except ConnectionRefusedError:
        print(f"Connection refused: {args.addr}", file=sys.stderr)
        sys.exit(1)


def cmd_describe(args: argparse.Namespace) -> None:
    """Describe a selector (metadata)."""
    host, port = _parse_addr(args.addr)
    try:
        with Session(host, port) as sess:
            resp = sess.describe(args.selector)
            if resp.is_error:
                print(f"Error {resp.verb}: {resp.body.strip()}", file=sys.stderr)
                sys.exit(1)
            for k, v in sorted(resp.headers.items()):
                print(f"{k}: {v}")
            if resp.body:
                print()
                print(resp.body.rstrip())
    except ProtocolError as e:
        print(f"Protocol error: {e}", file=sys.stderr)
        sys.exit(1)
    except ConnectionRefusedError:
        print(f"Connection refused: {args.addr}", file=sys.stderr)
        sys.exit(1)


# -- Main ----------------------------------------------------------------

def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(
        prog="rabbit",
        description="🐇 Rabbit — Python client for the Rabbit protocol",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    # browse
    p_browse = sub.add_parser("browse", help="Interactive menu browsing")
    p_browse.add_argument("addr", help="Burrow address (host:port)")
    p_browse.add_argument("-s", "--selector", default="/", help="Starting selector")
    p_browse.set_defaults(func=cmd_browse)

    # fetch
    p_fetch = sub.add_parser("fetch", help="Fetch a resource")
    p_fetch.add_argument("addr", help="Burrow address")
    p_fetch.add_argument("selector", help="Resource selector (e.g. /0/readme)")
    p_fetch.set_defaults(func=cmd_fetch)

    # list
    p_list = sub.add_parser("list", help="List a menu / directory")
    p_list.add_argument("addr", help="Burrow address")
    p_list.add_argument("selector", nargs="?", default="/", help="Menu selector")
    p_list.set_defaults(func=cmd_list)

    # sub
    p_sub = sub.add_parser("sub", help="Subscribe to an event topic")
    p_sub.add_argument("addr", help="Burrow address")
    p_sub.add_argument("topic", help="Event topic (e.g. /q/chat)")
    p_sub.add_argument("--since", type=int, default=0, help="Replay from seq")
    p_sub.set_defaults(func=cmd_sub)

    # pub
    p_pub = sub.add_parser("pub", help="Publish to an event topic")
    p_pub.add_argument("addr", help="Burrow address")
    p_pub.add_argument("topic", help="Event topic (e.g. /q/chat)")
    p_pub.add_argument("message", help="Message body")
    p_pub.set_defaults(func=cmd_pub)

    # describe
    p_desc = sub.add_parser("describe", help="Describe a selector")
    p_desc.add_argument("addr", help="Burrow address")
    p_desc.add_argument("selector", help="Resource selector")
    p_desc.set_defaults(func=cmd_describe)

    args = parser.parse_args(argv)
    args.func(args)


if __name__ == "__main__":
    main()
