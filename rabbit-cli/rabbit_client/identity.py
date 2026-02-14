"""Ed25519 identity for the Rabbit client.

Each client session generates an ephemeral Ed25519 keypair.
The public key is the Burrow-ID: ``ed25519:<hex(pubkey)>``.
"""

from __future__ import annotations

from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
    Ed25519PublicKey,
)


class Identity:
    """An Ed25519 identity (ephemeral per session)."""

    def __init__(self) -> None:
        self._private = Ed25519PrivateKey.generate()
        self._public: Ed25519PublicKey = self._private.public_key()
        self._pub_bytes: bytes = self._public.public_bytes_raw()

    @property
    def burrow_id(self) -> str:
        """Return ``ed25519:<hex>`` Burrow-ID string."""
        return "ed25519:" + self._pub_bytes.hex()

    def sign(self, data: bytes) -> bytes:
        """Sign *data* with our private key, return raw 64-byte signature."""
        return self._private.sign(data)

    def sign_hex(self, data: bytes) -> str:
        """Sign *data* and return ``ed25519:<hex(signature)>``."""
        sig = self.sign(data)
        return "ed25519:" + sig.hex()
