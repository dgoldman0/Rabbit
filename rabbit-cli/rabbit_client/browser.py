"""Interactive Rabbit browser.

Provides a Gopher-style menu navigation loop with a back-stack,
search, event subscription, and content viewing.
"""

from __future__ import annotations

import sys

from .menu import MenuItem, parse_menu
from .protocol import (
    STATUS_MOVED,
    STATUS_NOT_FOUND,
    TYPE_BINARY,
    TYPE_INFO,
    TYPE_MENU,
    TYPE_QUEUE,
    TYPE_SEARCH,
    TYPE_TEXT,
    TYPE_UI,
)
from .session import Session


# -- Terminal colours (ANSI) ---------------------------------------------

class _C:
    RESET  = "\033[0m"
    BOLD   = "\033[1m"
    DIM    = "\033[2m"
    CYAN   = "\033[36m"
    GREEN  = "\033[32m"
    YELLOW = "\033[33m"
    RED    = "\033[31m"
    MAGENTA = "\033[35m"
    BLUE   = "\033[34m"

def _header(text: str) -> str:
    return f"{_C.BOLD}{_C.CYAN}{text}{_C.RESET}"

def _info(text: str) -> str:
    return f"{_C.DIM}{text}{_C.RESET}"

def _err(text: str) -> str:
    return f"{_C.RED}{text}{_C.RESET}"

def _ok(text: str) -> str:
    return f"{_C.GREEN}{text}{_C.RESET}"

def _selector(text: str) -> str:
    return f"{_C.YELLOW}{text}{_C.RESET}"


# -- Browser -------------------------------------------------------------

class Browser:
    """Interactive Rabbit browser with back-stack navigation."""

    def __init__(self, session: Session) -> None:
        self.session = session
        self.nav_stack: list[str] = []
        self.current_selector = "/"
        self.current_items: list[MenuItem] = []

    def run(self, start_selector: str = "/") -> None:
        """Main browse loop."""
        self.current_selector = start_selector
        self._print_banner()

        while True:
            self._show_menu()
            try:
                cmd = input(f"\n{_C.BOLD}rabbit>{_C.RESET} ").strip()
            except (EOFError, KeyboardInterrupt):
                print()
                break

            if not cmd:
                continue

            if cmd in ("q", "quit", "exit"):
                break
            elif cmd in ("b", "back", ".."):
                self._go_back()
            elif cmd == "?":
                self._show_help()
            elif cmd.startswith("/"):
                # Inline search
                query = cmd[1:].strip()
                if query:
                    self._do_search(self.current_selector, query)
            elif cmd == "r":
                # Refresh
                continue
            elif cmd == "ping":
                self._do_ping()
            elif cmd == "info":
                self._do_describe(self.current_selector)
            elif cmd.startswith("info "):
                # Describe a specific item by number
                self._describe_item(cmd.split(None, 1)[1])
            elif cmd.isdigit():
                self._navigate_item(int(cmd))
            else:
                print(_err(f"  Unknown command: {cmd!r}  (type ? for help)"))

    # -- Menu display ----------------------------------------------------

    def _show_menu(self) -> None:
        """Fetch and display the current menu."""
        resp = self.session.list(self.current_selector)

        # Handle redirects
        if resp.status_code == STATUS_MOVED:
            loc = resp.get("Location")
            if loc:
                print(_info(f"  → Redirected to {loc}"))
                self.current_selector = loc
                return self._show_menu()

        if resp.is_error:
            print(_err(f"  Error {resp.verb}: {resp.body.strip()}"))
            if self.nav_stack:
                self._go_back()
            return

        self.current_items = parse_menu(resp.body)

        # Display
        print()
        loc_display = f"  {_selector(self.current_selector)}"
        if self.session.server_id:
            server = self.session.server_id
            if server.startswith("ed25519:"):
                server = server[8:][:12] + "…"
            loc_display = f"  {_C.DIM}@{server}{_C.RESET}  {_selector(self.current_selector)}"
        print(_header("─" * 60))
        print(loc_display)
        print(_header("─" * 60))

        nav_num = 0
        for item in self.current_items:
            if item.is_info:
                print(item.display())
            else:
                nav_num += 1
                print(item.display(number=nav_num))

        if not self.current_items:
            print(_info("  (empty menu)"))

    # -- Navigation ------------------------------------------------------

    def _navigate_item(self, num: int) -> None:
        """Navigate to a numbered menu item."""
        # Build list of navigable items
        navigable = [it for it in self.current_items if it.is_navigable]

        if num < 1 or num > len(navigable):
            print(_err(f"  No item #{num} (1–{len(navigable)})"))
            return

        item = navigable[num - 1]

        # Check for remote item
        if item.is_remote:
            print(_info(f"  Remote item on burrow {item.burrow}"))
            print(_info(f"  Use: rabbit browse {item.burrow} --selector {item.selector}"))
            return

        # Dispatch by type
        if item.type_code == TYPE_MENU:
            self.nav_stack.append(self.current_selector)
            self.current_selector = item.selector
        elif item.type_code == TYPE_TEXT:
            self._view_text(item)
        elif item.type_code == TYPE_SEARCH:
            self._prompt_search(item)
        elif item.type_code == TYPE_QUEUE:
            self._stream_events(item)
        elif item.type_code == TYPE_BINARY:
            print(_info("  (binary content — not displayed in terminal)"))
        elif item.type_code == TYPE_UI:
            self._view_text(item)  # Show raw UI declaration
        else:
            # Treat unknown as fetchable text
            self._view_text(item)

    def _go_back(self) -> None:
        """Pop nav stack and go to parent menu."""
        if self.nav_stack:
            self.current_selector = self.nav_stack.pop()
        else:
            print(_info("  (already at root)"))

    # -- Content viewers -------------------------------------------------

    def _view_text(self, item: MenuItem) -> None:
        """Fetch and display a text item."""
        print()
        print(_header(f"── {item.label} ──"))
        resp = self.session.fetch(item.selector)

        if resp.is_error:
            print(_err(f"  Error {resp.verb}: {resp.body.strip()}"))
            return

        # Display body with left margin
        for line in resp.body.rstrip().split("\n"):
            print(f"  {line.rstrip()}")

        # Show metadata
        view = resp.get("View")
        if view:
            print(_info(f"\n  [{view}]"))

        print()
        try:
            input(_info("  ↩ Press Enter to return "))
        except (EOFError, KeyboardInterrupt):
            pass

    def _prompt_search(self, item: MenuItem) -> None:
        """Prompt for a search query and display results."""
        try:
            query = input(f"\n  {_C.BOLD}Search:{_C.RESET} ").strip()
        except (EOFError, KeyboardInterrupt):
            return

        if not query:
            return

        self._do_search(item.selector, query)

    def _do_search(self, selector: str, query: str) -> None:
        """Execute a search and display results as a navigable sub-menu."""
        # Use /7/ path if selector doesn't look like a search endpoint
        if not selector.startswith("/7"):
            selector = "/7" + selector

        resp = self.session.search(selector, query)

        if resp.is_error:
            print(_err(f"  Search error: {resp.body.strip()}"))
            return

        results = parse_menu(resp.body)
        if not results:
            print(_info("  No results found."))
            return

        # Display results as a temporary menu
        print(_header(f"\n  Search results for: {query!r}"))
        nav_num = 0
        for item in results:
            if item.is_info:
                print(item.display())
            else:
                nav_num += 1
                print(item.display(number=nav_num))

        # Allow navigating into a result
        try:
            choice = input(f"\n  {_C.BOLD}#{_C.RESET} ").strip()
        except (EOFError, KeyboardInterrupt):
            return

        if choice.isdigit():
            navigable = [it for it in results if it.is_navigable]
            n = int(choice)
            if 1 <= n <= len(navigable):
                item = navigable[n - 1]
                if item.type_code == TYPE_MENU:
                    self.nav_stack.append(self.current_selector)
                    self.current_selector = item.selector
                elif item.type_code == TYPE_TEXT:
                    self._view_text(item)
                else:
                    self._view_text(item)

    def _stream_events(self, item: MenuItem) -> None:
        """Subscribe to an event stream and display events."""
        print(_header(f"\n  ⚡ Subscribing to {item.selector}"))
        print(_info("  (Ctrl-C to stop)\n"))

        resp = self.session.subscribe(item.selector)

        if resp.is_error:
            print(_err(f"  Subscribe error: {resp.body.strip()}"))
            return

        if resp.is_success or resp.status_code == "201":
            print(_ok("  Subscribed. Waiting for events...\n"))

        try:
            while True:
                ev = self.session.recv_event(timeout=60.0)
                if ev is None:
                    print(_info("  (connection closed)"))
                    break
                if ev.is_event:
                    seq = ev.get("Seq", "?")
                    ts = ev.get("Timestamp", "")
                    body = ev.body.rstrip()
                    ts_display = f"  {_C.DIM}{ts}{_C.RESET}" if ts else ""
                    print(f"  {_C.DIM}#{seq}{_C.RESET}{ts_display}  {body}")
                elif ev.verb == "PING":
                    # Auto-respond to keepalive
                    pass
                else:
                    # Other frames (heartbeats, etc.)
                    pass
        except KeyboardInterrupt:
            print(_info("\n  Unsubscribed."))

    # -- Utilities -------------------------------------------------------

    def _do_ping(self) -> None:
        resp = self.session.ping()
        if resp.is_success:
            print(_ok("  PONG"))
        else:
            print(_err(f"  Ping failed: {resp.verb}"))

    def _do_describe(self, selector: str) -> None:
        """Describe a selector (metadata)."""
        resp = self.session.describe(selector)
        if resp.is_error:
            print(_err(f"  Error: {resp.body.strip()}"))
            return
        print(_header(f"\n  Metadata for {selector}"))
        for k, v in sorted(resp.headers.items()):
            print(f"  {_C.DIM}{k}:{_C.RESET} {v}")

    def _describe_item(self, num_str: str) -> None:
        """Describe a specific menu item by number."""
        if not num_str.isdigit():
            print(_err(f"  Usage: info <number>"))
            return
        navigable = [it for it in self.current_items if it.is_navigable]
        n = int(num_str)
        if n < 1 or n > len(navigable):
            print(_err(f"  No item #{n}"))
            return
        item = navigable[n - 1]
        self._do_describe(item.selector)

    def _print_banner(self) -> None:
        """Print welcome banner."""
        print()
        print(_header("  🐇  Rabbit Client"))
        addr = f"{self.session.host}:{self.session.port}"
        status = _ok("authenticated") if self.session.authenticated else _info("anonymous")
        print(f"  Connected to {_C.BOLD}{addr}{_C.RESET}  ({status})")
        if self.session.server_id and self.session.server_id != "anonymous":
            sid = self.session.server_id
            if sid.startswith("ed25519:"):
                sid = sid[8:][:16] + "…"
            print(f"  Server: {_C.DIM}{sid}{_C.RESET}")
        print()

    def _show_help(self) -> None:
        """Print interactive help."""
        print(_header("\n  Commands:"))
        print(f"  {_C.BOLD}<number>{_C.RESET}    Navigate to menu item")
        print(f"  {_C.BOLD}b{_C.RESET}           Go back")
        print(f"  {_C.BOLD}/query{_C.RESET}      Search")
        print(f"  {_C.BOLD}r{_C.RESET}           Refresh current menu")
        print(f"  {_C.BOLD}info{_C.RESET}        Describe current selector")
        print(f"  {_C.BOLD}info N{_C.RESET}      Describe menu item N")
        print(f"  {_C.BOLD}ping{_C.RESET}        Send keepalive")
        print(f"  {_C.BOLD}q{_C.RESET}           Quit")
        print()
