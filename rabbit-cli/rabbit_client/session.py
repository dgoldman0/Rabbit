"""Rabbit session: handshake, request/response helpers.

Manages the HELLO → CHALLENGE → AUTH handshake and provides
high-level methods like ``list()``, ``fetch()``, ``subscribe()``
that send frames and return parsed responses.
"""

from __future__ import annotations

from .identity import Identity
from .protocol import (
    Frame,
    ProtocolError,
    TxnCounter,
    auth_frame,
    describe_frame,
    fetch_frame,
    hello_frame,
    list_frame,
    ping_frame,
    publish_frame,
    search_frame,
    subscribe_frame,
    HDR_BURROW_ID,
    HDR_NONCE,
    HDR_SESSION_TOKEN,
    STATUS_AUTH_REQUIRED,
    STATUS_CHALLENGE,
    STATUS_MOVED,
)
from .transport import Tunnel


class Session:
    """A connected, authenticated session to a Rabbit burrow."""

    def __init__(self, host: str, port: int, timeout: float = 10.0) -> None:
        self.host = host
        self.port = port
        self.identity = Identity()
        self.tunnel = Tunnel(host, port, timeout=timeout)
        self.txn = TxnCounter()

        # Set during handshake
        self.server_id: str = ""
        self.session_token: str = ""
        self.server_caps: str = ""
        self.authenticated: bool = False

    # -- Connection lifecycle --------------------------------------------

    def connect(self) -> None:
        """Open the tunnel and perform the HELLO handshake."""
        self.tunnel.connect()
        self._handshake()

    def close(self) -> None:
        """Close the session."""
        self.tunnel.close()

    def __enter__(self) -> "Session":
        self.connect()
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    # -- Request helpers -------------------------------------------------

    def list(self, selector: str = "/") -> Frame:
        """Send LIST and return the response frame."""
        f = list_frame(selector, txn=self.txn.next())
        return self._request(f)

    def fetch(self, selector: str) -> Frame:
        """Send FETCH and return the response frame."""
        f = fetch_frame(selector, txn=self.txn.next())
        return self._request(f)

    def search(self, selector: str, query: str) -> Frame:
        """Send SEARCH and return the response frame."""
        f = search_frame(selector, query, txn=self.txn.next())
        return self._request(f)

    def subscribe(self, topic: str, since: str = "") -> Frame:
        """Send SUBSCRIBE and return the initial response.

        After this, call ``recv_event()`` to stream events.
        """
        f = subscribe_frame(topic, txn=self.txn.next(), since=since)
        return self._request(f)

    def recv_event(self, timeout: float | None = None) -> Frame | None:
        """Read the next event frame from the tunnel.

        Returns ``None`` on clean EOF or timeout.
        """
        try:
            return self.tunnel.recv_frame(timeout=timeout)
        except (ProtocolError, OSError):
            return None

    def publish(self, topic: str, message: str) -> Frame:
        """Send PUBLISH and return the response."""
        f = publish_frame(topic, message, txn=self.txn.next())
        return self._request(f)

    def describe(self, selector: str) -> Frame:
        """Send DESCRIBE and return the response."""
        f = describe_frame(selector, txn=self.txn.next())
        return self._request(f)

    def ping(self) -> Frame:
        """Send PING and return PONG."""
        return self._request(ping_frame())

    # -- Internal --------------------------------------------------------

    def _request(self, frame: Frame) -> Frame:
        """Send a frame and wait for one response."""
        self.tunnel.send_frame(frame)
        resp = self.tunnel.recv_frame()
        if resp is None:
            raise ProtocolError("Connection closed (no response)")
        return resp

    def _handshake(self) -> None:
        """Perform HELLO → optional CHALLENGE/AUTH → 200 HELLO."""
        hello = hello_frame(self.identity.burrow_id)
        self.tunnel.send_frame(hello)

        resp = self.tunnel.recv_frame()
        if resp is None:
            raise ProtocolError("Connection closed during handshake")

        # --- Authenticated path: 300 CHALLENGE → AUTH → 200 ---
        if resp.status_code == STATUS_CHALLENGE:
            nonce_hex = resp.get(HDR_NONCE)
            if not nonce_hex:
                raise ProtocolError("CHALLENGE without Nonce")
            nonce_bytes = bytes.fromhex(nonce_hex)
            proof = self.identity.sign_hex(nonce_bytes)

            auth = auth_frame(proof)
            self.tunnel.send_frame(auth)

            resp = self.tunnel.recv_frame()
            if resp is None:
                raise ProtocolError("Connection closed during auth")
            self.authenticated = True

        # --- Anonymous path: immediate 200 ---
        if resp.status_code == STATUS_AUTH_REQUIRED:
            raise ProtocolError(f"Server requires auth but rejected us: {resp.body}")

        if not resp.is_success:
            raise ProtocolError(
                f"Handshake failed: {resp.verb} {' '.join(resp.args)} — {resp.body}"
            )

        self.server_id = resp.get(HDR_BURROW_ID)
        self.session_token = resp.get(HDR_SESSION_TOKEN)
        self.server_caps = resp.get("Caps")

    def _follow_redirect(self, resp: Frame) -> str | None:
        """Extract the Location path from a 301 MOVED response."""
        if resp.status_code == STATUS_MOVED:
            return resp.get("Location")
        return None
