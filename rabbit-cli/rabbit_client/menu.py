"""Rabbitmap menu parser.

Menus are tab-delimited lines with CRLF endings:

    <type><label>\t<selector>\t<burrow>\t<hint>\r\n

Terminated by a line containing only ``.``.
"""

from __future__ import annotations

from dataclasses import dataclass

from .protocol import TYPE_GLYPHS, TYPE_INFO


@dataclass
class MenuItem:
    """A single item in a Rabbit menu (rabbitmap)."""

    type_code: str       # '0', '1', '7', '9', 'q', 'i', etc.
    label: str           # Human-readable display text
    selector: str        # Path to resource (e.g. '/0/readme')
    burrow: str          # '=' for local, or burrow-id/host for remote
    hint: str = ""       # Optional metadata

    @property
    def is_info(self) -> bool:
        return self.type_code == TYPE_INFO

    @property
    def is_navigable(self) -> bool:
        return self.type_code != TYPE_INFO

    @property
    def is_remote(self) -> bool:
        return self.burrow not in ("=", "")

    @property
    def glyph(self) -> str:
        return TYPE_GLYPHS.get(self.type_code, "❓")

    def display(self, number: int | None = None) -> str:
        """Format for terminal display.

        Info lines get no number; navigable items get ``[N]``.
        """
        prefix = f"  [{number:>2}]" if number is not None else "     "
        remote = f"  @ {self.burrow}" if self.is_remote else ""
        return f"{prefix} {self.glyph}  {self.label}{remote}"


def parse_menu(body: str) -> list[MenuItem]:
    """Parse a rabbitmap body into a list of ``MenuItem``."""
    items: list[MenuItem] = []

    for raw_line in body.split("\n"):
        line = raw_line.rstrip("\r")
        if not line or line == ".":
            continue

        # Split on tabs: type+label \t selector \t burrow \t hint
        parts = line.split("\t", 3)

        if not parts or not parts[0]:
            continue

        # First character is the type code, rest is the label
        type_code = parts[0][0]
        label = parts[0][1:]

        selector = parts[1] if len(parts) > 1 else ""
        burrow = parts[2] if len(parts) > 2 else "="
        hint = parts[3] if len(parts) > 3 else ""

        items.append(MenuItem(
            type_code=type_code,
            label=label,
            selector=selector,
            burrow=burrow or "=",
            hint=hint,
        ))

    return items
