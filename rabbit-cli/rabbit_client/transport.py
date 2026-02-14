"""TLS transport and frame I/O for the Rabbit protocol.

Handles TLS 1.3 connections with ALPN ``rabbit/1``, reading and
writing complete frames from the wire.  No async — plain blocking
sockets wrapped in ``ssl``.
"""

from __future__ import annotations

import socket
import ssl
from typing import Optional

from .protocol import (
    ALPN_PROTOCOL,
    END_MARKER,
    HDR_LENGTH,
    Frame,
    ProtocolError,
)


def _make_ssl_context() -> ssl.SSLContext:
    """Create an SSL context that mirrors the Rust client behaviour.

    - TLS 1.3 minimum
    - Certificate verification disabled (trust is at the protocol
      layer via Ed25519 TOFU)
    - ALPN: ``rabbit/1``
    """
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    ctx.minimum_version = ssl.TLSVersion.TLSv1_3
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    ctx.set_alpn_protocols([ALPN_PROTOCOL.decode()])
    return ctx


class Tunnel:
    """A TLS tunnel to a Rabbit burrow.

    Wraps a plain TCP socket upgraded to TLS.  Provides ``send_frame``
    and ``recv_frame`` for reading/writing Rabbit frames.
    """

    def __init__(self, host: str, port: int, timeout: float = 10.0) -> None:
        self.host = host
        self.port = port
        self._timeout = timeout
        self._sock: ssl.SSLSocket | None = None
        self._buf = b""

    # -- Connection lifecycle --------------------------------------------

    def connect(self) -> None:
        """Open a TLS tunnel to the burrow."""
        raw = socket.create_connection((self.host, self.port), timeout=self._timeout)
        ctx = _make_ssl_context()
        self._sock = ctx.wrap_socket(raw, server_hostname=self.host)
        self._sock.settimeout(self._timeout)

    def close(self) -> None:
        """Close the tunnel gracefully."""
        if self._sock:
            try:
                self._sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            self._sock.close()
            self._sock = None

    @property
    def connected(self) -> bool:
        return self._sock is not None

    # -- Frame I/O -------------------------------------------------------

    def send_frame(self, frame: Frame) -> None:
        """Serialize and send a frame over the tunnel."""
        if not self._sock:
            raise ProtocolError("Not connected")
        data = frame.serialize()
        self._sock.sendall(data)

    def recv_frame(self, timeout: float | None = None) -> Optional[Frame]:
        """Read a complete frame from the tunnel.

        Returns ``None`` on clean EOF.  Raises on protocol errors.
        """
        if not self._sock:
            raise ProtocolError("Not connected")

        old_timeout = self._sock.gettimeout()
        if timeout is not None:
            self._sock.settimeout(timeout)
        try:
            return self._read_frame()
        finally:
            if timeout is not None:
                self._sock.settimeout(old_timeout)

    # -- Internal --------------------------------------------------------

    def _read_frame(self) -> Optional[Frame]:
        """Low-level frame reader: accumulates bytes until ``End:\\r\\n``
        then reads the body based on ``Length`` header.
        """
        end_marker = END_MARKER.encode("utf-8")

        # Accumulate until we see the end marker
        while end_marker not in self._buf:
            chunk = self._recv()
            if not chunk:
                if not self._buf:
                    return None  # clean EOF
                raise ProtocolError("Connection closed mid-frame")
            self._buf += chunk

        # Split header section from the rest
        idx = self._buf.index(end_marker) + len(end_marker)
        header_part = self._buf[:idx]

        # Check if there's a Length header to determine body size
        body_len = self._extract_length(header_part)

        # Read body bytes
        total_needed = idx + body_len
        while len(self._buf) < total_needed:
            chunk = self._recv()
            if not chunk:
                raise ProtocolError("Connection closed mid-body")
            self._buf += chunk

        raw = self._buf[:total_needed]
        self._buf = self._buf[total_needed:]

        text = raw.decode("utf-8", errors="replace")
        return Frame.parse(text)

    def _recv(self, size: int = 8192) -> bytes:
        """Receive up to *size* bytes, returning empty on EOF."""
        assert self._sock is not None
        try:
            return self._sock.recv(size)
        except (ConnectionResetError, BrokenPipeError):
            return b""

    @staticmethod
    def _extract_length(header_bytes: bytes) -> int:
        """Scan the header section for a Length: header value."""
        text = header_bytes.decode("utf-8", errors="replace")
        for line in text.split("\r\n"):
            if line.lower().startswith("length:"):
                _, val = line.split(":", 1)
                return int(val.strip())
        return 0
