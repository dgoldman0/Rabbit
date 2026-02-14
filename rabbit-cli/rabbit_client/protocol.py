"""Rabbit wire protocol: Frame serialization, parsing, and constants.

Every Rabbit frame is UTF-8 text with CRLF line endings:

    <VERB> [<args>...]\r\n
    <Header>: <Value>\r\n
    End:\r\n
    [<body>]
"""

from __future__ import annotations

from dataclasses import dataclass, field

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

CRLF = "\r\n"
END_MARKER = "End:" + CRLF
PROTOCOL_VERSION = "RABBIT/1.0"
ALPN_PROTOCOL = b"rabbit/1"

# Verbs — client → server
VERB_HELLO = "HELLO"
VERB_AUTH = "AUTH"
VERB_LIST = "LIST"
VERB_FETCH = "FETCH"
VERB_SEARCH = "SEARCH"
VERB_SUBSCRIBE = "SUBSCRIBE"
VERB_PUBLISH = "PUBLISH"
VERB_DESCRIBE = "DESCRIBE"
VERB_PING = "PING"
VERB_CREDIT = "CREDIT"
VERB_ACK = "ACK"
VERB_DELEGATE = "DELEGATE"
VERB_OFFER = "OFFER"

# Response codes
STATUS_OK = "200"
STATUS_SUBSCRIBED = "201"
STATUS_DONE = "204"
STATUS_EVENT = "210"
STATUS_CHALLENGE = "300"
STATUS_MOVED = "301"
STATUS_BAD_REQUEST = "400"
STATUS_FORBIDDEN = "403"
STATUS_NOT_FOUND = "404"
STATUS_TIMEOUT = "408"
STATUS_OUT_OF_ORDER = "409"
STATUS_FLOW_LIMIT = "429"
STATUS_BAD_HELLO = "431"
STATUS_AUTH_REQUIRED = "440"
STATUS_CANCELED = "499"
STATUS_BUSY = "503"
STATUS_INTERNAL_ERROR = "520"

# Selector type codes
TYPE_TEXT = "0"
TYPE_MENU = "1"
TYPE_SEARCH = "7"
TYPE_BINARY = "9"
TYPE_QUEUE = "q"
TYPE_UI = "u"
TYPE_INFO = "i"

# Display glyphs for menu item types
TYPE_GLYPHS: dict[str, str] = {
    TYPE_TEXT: "📄",
    TYPE_MENU: "📂",
    TYPE_SEARCH: "🔍",
    TYPE_BINARY: "📦",
    TYPE_QUEUE: "⚡",
    TYPE_UI: "🖥 ",
    TYPE_INFO: "ℹ️ ",
}

# Headers
HDR_LANE = "Lane"
HDR_TXN = "Txn"
HDR_SEQ = "Seq"
HDR_ACK = "ACK"
HDR_CREDIT = "Credit"
HDR_LENGTH = "Length"
HDR_VIEW = "View"
HDR_ACCEPT_VIEW = "Accept-View"
HDR_PART = "Part"
HDR_IDEM = "Idem"
HDR_TIMEOUT = "Timeout"
HDR_QOS = "QoS"
HDR_BURROW_ID = "Burrow-ID"
HDR_NONCE = "Nonce"
HDR_PROOF = "Proof"
HDR_SESSION_TOKEN = "Session-Token"
HDR_CAPS = "Caps"
HDR_SINCE = "Since"
HDR_LOCATION = "Location"
HDR_TIMESTAMP = "Timestamp"
HDR_HEARTBEATS = "Heartbeats"


# ---------------------------------------------------------------------------
# Frame
# ---------------------------------------------------------------------------

@dataclass
class Frame:
    """A single Rabbit protocol frame."""

    verb: str = ""
    args: list[str] = field(default_factory=list)
    headers: dict[str, str] = field(default_factory=dict)
    body: str = ""

    # -- Convenience properties ------------------------------------------

    @property
    def status_code(self) -> str | None:
        """If this is a response frame, return the numeric status code."""
        if self.verb and self.verb[0].isdigit():
            return self.verb
        return None

    @property
    def status_label(self) -> str:
        """Return the human label from args (e.g. 'MENU' from '200 MENU')."""
        return self.args[0] if self.args else ""

    @property
    def is_success(self) -> bool:
        code = self.status_code
        return code is not None and code.startswith("2")

    @property
    def is_error(self) -> bool:
        code = self.status_code
        return code is not None and int(code) >= 400

    @property
    def is_event(self) -> bool:
        return self.verb in ("EVENT", STATUS_EVENT)

    # -- Header helpers --------------------------------------------------

    def get(self, key: str, default: str = "") -> str:
        return self.headers.get(key, default)

    def lane(self) -> int:
        return int(self.headers.get(HDR_LANE, "0"))

    def txn(self) -> str:
        return self.headers.get(HDR_TXN, "")

    def seq(self) -> int:
        return int(self.headers.get(HDR_SEQ, "0"))

    def length(self) -> int:
        return int(self.headers.get(HDR_LENGTH, "0"))

    # -- Serialization ---------------------------------------------------

    def serialize(self) -> bytes:
        """Serialize the frame to wire bytes (UTF-8 with CRLF)."""
        parts: list[str] = []

        # Start line: VERB arg1 arg2 ...
        start = self.verb
        if self.args:
            start += " " + " ".join(self.args)
        parts.append(start + CRLF)

        # Headers (sorted for deterministic output)
        for key in sorted(self.headers):
            parts.append(f"{key}: {self.headers[key]}{CRLF}")

        # End marker
        parts.append(END_MARKER)

        # Body
        if self.body:
            parts.append(self.body)

        return "".join(parts).encode("utf-8")

    def set_body(self, body: str) -> None:
        """Set the body and auto-update the Length header."""
        self.body = body
        self.headers[HDR_LENGTH] = str(len(body.encode("utf-8")))

    # -- Parsing ---------------------------------------------------------

    @staticmethod
    def parse(data: str) -> Frame:
        """Parse a frame from a raw string (with CRLF line endings)."""
        # Split header section from body at 'End:\r\n'
        end_idx = data.find(END_MARKER)
        if end_idx == -1:
            # Try without \r for tolerance
            end_idx = data.find("End:\n")
            if end_idx == -1:
                raise ProtocolError(f"Missing End: marker in frame: {data[:80]!r}")
            header_section = data[:end_idx]
            body_section = data[end_idx + len("End:\n"):]
        else:
            header_section = data[:end_idx]
            body_section = data[end_idx + len(END_MARKER):]

        # Parse header lines
        lines = header_section.replace("\r\n", "\n").split("\n")
        lines = [l for l in lines if l]  # drop empties

        if not lines:
            raise ProtocolError("Empty frame")

        # Start line
        start_parts = lines[0].split(None)  # split on whitespace
        verb = start_parts[0] if start_parts else ""
        args = start_parts[1:] if len(start_parts) > 1 else []

        # Headers
        headers: dict[str, str] = {}
        for line in lines[1:]:
            colon = line.find(":")
            if colon != -1:
                key = line[:colon].strip()
                value = line[colon + 1:].strip()
                headers[key] = value

        # Body — respect Length header if present
        body = ""
        if HDR_LENGTH in headers:
            length = int(headers[HDR_LENGTH])
            body = body_section[:length]
        elif body_section:
            body = body_section

        return Frame(verb=verb, args=args, headers=headers, body=body)


# ---------------------------------------------------------------------------
# Builders — convenience constructors for common frames
# ---------------------------------------------------------------------------

def hello_frame(burrow_id: str) -> Frame:
    """Build a HELLO RABBIT/1.0 frame."""
    f = Frame(verb=VERB_HELLO, args=[PROTOCOL_VERSION])
    f.headers[HDR_BURROW_ID] = burrow_id
    f.headers[HDR_CAPS] = "lanes,async"
    return f


def auth_frame(proof: str) -> Frame:
    """Build an AUTH PROOF frame."""
    f = Frame(verb=VERB_AUTH, args=["PROOF"])
    f.headers[HDR_PROOF] = proof
    return f


def list_frame(selector: str, lane: int = 0, txn: str = "") -> Frame:
    """Build a LIST frame."""
    f = Frame(verb=VERB_LIST, args=[selector])
    f.headers[HDR_LANE] = str(lane)
    if txn:
        f.headers[HDR_TXN] = txn
    return f


def fetch_frame(selector: str, lane: int = 0, txn: str = "") -> Frame:
    """Build a FETCH frame."""
    f = Frame(verb=VERB_FETCH, args=[selector])
    f.headers[HDR_LANE] = str(lane)
    if txn:
        f.headers[HDR_TXN] = txn
    return f


def search_frame(selector: str, query: str, lane: int = 0, txn: str = "") -> Frame:
    """Build a SEARCH frame with query in the body."""
    f = Frame(verb=VERB_SEARCH, args=[selector])
    f.headers[HDR_LANE] = str(lane)
    if txn:
        f.headers[HDR_TXN] = txn
    f.set_body(query)
    return f


def subscribe_frame(topic: str, lane: int = 0, txn: str = "",
                     since: str = "") -> Frame:
    """Build a SUBSCRIBE frame."""
    f = Frame(verb=VERB_SUBSCRIBE, args=[topic])
    f.headers[HDR_LANE] = str(lane)
    if txn:
        f.headers[HDR_TXN] = txn
    if since:
        f.headers[HDR_SINCE] = since
    return f


def publish_frame(topic: str, message: str, lane: int = 0,
                   txn: str = "") -> Frame:
    """Build a PUBLISH frame."""
    f = Frame(verb=VERB_PUBLISH, args=[topic])
    f.headers[HDR_LANE] = str(lane)
    if txn:
        f.headers[HDR_TXN] = txn
    f.set_body(message)
    return f


def describe_frame(selector: str, lane: int = 0, txn: str = "") -> Frame:
    """Build a DESCRIBE frame."""
    f = Frame(verb=VERB_DESCRIBE, args=[selector])
    f.headers[HDR_LANE] = str(lane)
    if txn:
        f.headers[HDR_TXN] = txn
    return f


def ping_frame() -> Frame:
    """Build a PING frame."""
    f = Frame(verb=VERB_PING)
    f.headers[HDR_LANE] = "0"
    return f


# ---------------------------------------------------------------------------
# Transaction counter
# ---------------------------------------------------------------------------

class TxnCounter:
    """Thread-safe transaction ID generator: T-1, T-2, ..."""

    def __init__(self) -> None:
        self._n = 0

    def next(self) -> str:
        self._n += 1
        return f"T-{self._n}"


# ---------------------------------------------------------------------------
# Errors
# ---------------------------------------------------------------------------

class ProtocolError(Exception):
    """Raised when the wire data doesn't conform to the Rabbit protocol."""
