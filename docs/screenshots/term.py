"""A headless terminal: run a program in a pty, emulate the screen with pyte, render it.

Used by ``herdr_demo.py`` to capture herdr itself (tab bar, sidebar, panes, popups) as
PNG frames. Rendering goes pyte screen -> Rich ``Text`` -> Rich SVG -> ``rsvg-convert``.
"""

from __future__ import annotations

import io
import re
import os
import pty
import select
import signal
import struct
import subprocess
import termios
import fcntl
import threading
import time
from pathlib import Path

import pyte
from rich.console import Console
from rich.style import Style
from rich.terminal_theme import TerminalTheme
from rich.text import Text

# Catppuccin Mocha: herdr's default theme, so default-coloured cells match its panels.
BG, FG = (30, 30, 46), (205, 214, 244)
ANSI = [
    (69, 71, 90), (243, 139, 168), (166, 227, 161), (249, 226, 175),
    (137, 180, 250), (245, 194, 231), (148, 226, 213), (186, 194, 222),
    (88, 91, 112), (243, 139, 168), (166, 227, 161), (249, 226, 175),
    (137, 180, 250), (245, 194, 231), (148, 226, 213), (166, 173, 200),
]
THEME = TerminalTheme(BG, FG, ANSI[:8], ANSI[8:])
NAMES = ["black", "red", "green", "brown", "blue", "magenta", "cyan", "white"]


def _hex(rgb: tuple[int, int, int]) -> str:
    return "#%02x%02x%02x" % rgb


def _color(value: str, default: tuple[int, int, int]) -> str:
    if value == "default":
        return _hex(default)
    if value.startswith("bright") and value[6:] in NAMES:
        return _hex(ANSI[8 + NAMES.index(value[6:])])
    if value in NAMES:
        return _hex(ANSI[NAMES.index(value)])
    if value == "yellow":
        return _hex(ANSI[3])
    if len(value) == 6:
        return "#" + value
    return _hex(default)


class _Screen(pyte.Screen):
    def __init__(self, *args):
        super().__init__(*args)
        self.replies: list[str] = []

    def report_device_status(self, mode: int, **kwargs) -> None:
        if not kwargs.get("private"):  # pyte has no private (DEC) status reports
            super().report_device_status(mode)

    def select_graphic_rendition(self, *attrs, **kwargs) -> None:
        super().select_graphic_rendition(*attrs)  # some TUIs send private SGR; ignore the flag

    def write_process_input(self, data: str) -> None:
        self.replies.append(data)


class Terminal:
    """``argv`` running in a ``cols`` x ``rows`` pty whose output feeds a pyte screen."""

    def __init__(self, argv: list[str], env: dict[str, str], cols: int, rows: int, cwd: str | None = None):
        self.cols, self.rows = cols, rows
        self.screen = _Screen(cols, rows)
        self.stream = pyte.ByteStream(self.screen)
        self.lock = threading.Lock()
        self.log = open(os.environ["TERM_LOG"], "wb") if os.environ.get("TERM_LOG") else None
        pid, fd = pty.fork()
        if pid == 0:  # child: size the pty before the program looks at it
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
            if cwd:
                os.chdir(cwd)
            os.execvpe(argv[0], argv, env)
        self.pid, self.fd = pid, fd
        self.alive = True
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _read(self) -> None:
        while self.alive:
            r, _, _ = select.select([self.fd], [], [], 0.1)
            if not r:
                continue
            try:
                data = os.read(self.fd, 65536)
            except OSError:
                break
            if not data:
                break
            if self.log:
                self.log.write(data)
            # kitty keyboard protocol push/pop/query: pyte does not know them and prints them
            data = re.sub(rb"\x1b\[[<>=?][0-9;]*u", b"", data)
            with self.lock:
                self.stream.feed(data)
                replies, self.screen.replies = self.screen.replies, []
            for reply in replies:  # cursor-position / device-attribute queries
                os.write(self.fd, reply.encode())

    def send(self, data: str | bytes, pause: float = 0.15) -> None:
        os.write(self.fd, data.encode() if isinstance(data, str) else data)
        time.sleep(pause)

    def text(self) -> str:
        with self.lock:
            return "\n".join(self.screen.display)

    def wait_for(self, needle: str, timeout: float = 20.0) -> None:
        end = time.time() + timeout
        while time.time() < end:
            if needle in self.text():
                return
            time.sleep(0.1)
        raise TimeoutError(f"{needle!r} not on screen:\n{self.text()}")

    def wait_gone(self, needle: str, timeout: float = 20.0) -> None:
        end = time.time() + timeout
        while time.time() < end:
            if needle not in self.text():
                return
            time.sleep(0.1)
        raise TimeoutError(f"{needle!r} still on screen")

    def rich_lines(self) -> list[Text]:
        lines = []
        with self.lock:
            buf = self.screen.buffer
            for y in range(self.rows):
                row = buf[y]
                line = Text()
                for x in range(self.cols):
                    ch = row[x]
                    if ch.data == "":
                        continue  # second cell of a wide character
                    fg, bg = _color(ch.fg, FG), _color(ch.bg, BG)
                    if ch.reverse:
                        fg, bg = bg, fg
                    line.append(ch.data or " ", Style(color=fg, bgcolor=bg, bold=ch.bold,
                                                      italic=ch.italics, underline=ch.underscore))
                lines.append(line)
        return lines

    def svg(self, title: str, lines: list[Text] | None = None, cols: int | None = None) -> str:
        console = Console(record=True, width=cols or self.cols, color_system="truecolor", file=io.StringIO(),
                          force_terminal=True, legacy_windows=False)
        for line in self.rich_lines() if lines is None else lines:
            console.print(line, no_wrap=True, overflow="crop", end="\n")
        svg = console.export_svg(title=title, theme=THEME)
        # rsvg-convert draws emoji as monochrome outlines; give ❓ its emoji red
        return svg.replace("❓", '<tspan style="fill:#e5484d;font-weight:bold">❓</tspan>')

    def png(self, path: Path, title: str, width: int = 1840, lines: list[Text] | None = None,
            cols: int | None = None) -> None:
        svg = path.with_suffix(".svg")
        svg.write_text(self.svg(title, lines, cols))
        subprocess.run(["rsvg-convert", "-w", str(width), "-o", str(path), str(svg)], check=True)
        svg.unlink()

    def close(self) -> None:
        self.alive = False
        try:
            os.kill(self.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            os.waitpid(self.pid, 0)
        except ChildProcessError:
            pass
        os.close(self.fd)
