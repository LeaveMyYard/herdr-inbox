"""The part of herdr-panorama's tests/scenario.py that tests/test_bridge_rules.py needs, copied
as it is (herdr-panorama fix/bridge-rules abc1d7e), so both plugins build the shared rule table
from the same captured records.

Every record is a deep copy of a template captured from a real file
(tests/fixtures/templates.json, copied verbatim from herdr-panorama; see its
tests/fixtures/capture.py) with ids, times and text filled in. Nothing about the wire format is
invented here.
"""

from __future__ import annotations

import copy
import json
import uuid
from datetime import datetime, timezone
from pathlib import Path

FIX = Path(__file__).parent / "fixtures"
T = json.loads((FIX / "templates.json").read_text())

BASE = datetime(2026, 10, 14, 9, 0, tzinfo=timezone.utc).timestamp()
CWD = "/home/user/project"
SLUG = "-home-user-project"


def iso(t: float) -> str:
    return datetime.fromtimestamp(t, timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.") + "%03dZ" % int((t % 1) * 1000)


def tpl(kind: str) -> dict:
    return copy.deepcopy(T[kind])


def stamp(rec: dict, t: float, session: str, agent_id: str | None = None) -> dict:
    rec["timestamp"] = iso(t)
    rec["uuid"] = str(uuid.uuid4())
    for key in ("sessionId", "session_id"):
        if key in rec:
            rec[key] = session
    if "cwd" in rec:
        rec["cwd"] = CWD
    if agent_id is not None:
        rec["agentId"] = agent_id
    return rec


class Transcript:
    """Writes one transcript (main session or an agent's own file)."""

    def __init__(self, path: Path, session: str, agent_id: str | None = None):
        self.path, self.session, self.agent_id = path, session, agent_id
        path.parent.mkdir(parents=True, exist_ok=True)
        self.lines: list[str] = []
        self._n = 0

    def add(self, rec: dict, t: float) -> None:
        self.lines.append(json.dumps(stamp(rec, t, self.session, self.agent_id), separators=(",", ":")))

    def raw(self, line: str) -> None:
        self.lines.append(line)

    def prompt(self, t: float, text: str, legacy: bool = False) -> None:
        """An operator prompt (``origin.kind == "human"``; ``legacy``: an older record without origin)."""
        rec = tpl("sub_first_user" if self.agent_id else ("user_prompt" if legacy else "user_prompt_human"))
        rec["message"]["content"] = text
        if legacy:
            rec.pop("origin", None)
        self.add(rec, t)

    def text(self, t: float, text: str) -> None:
        rec = tpl("sub_assistant_text" if self.agent_id else "assistant_text")
        block = next(b for b in rec["message"]["content"] if b.get("type") == "text")
        block["text"] = text
        rec["message"]["content"] = [block]
        self.add(rec, t)

    def write(self, partial: str | None = None) -> None:
        body = "\n".join(self.lines) + "\n"
        if partial:
            body += partial  # a last line still being written
        self.path.write_text(body)


class Project:
    def __init__(self, root: Path):
        self.root = root
        self.dir = root / SLUG
        self.dir.mkdir(parents=True, exist_ok=True)

    def main(self, session: str) -> Transcript:
        return Transcript(self.dir / f"{session}.jsonl", session)

    def agent(self, session: str, agent_id: str) -> Transcript:
        """An agent's own transcript under ``<session>/subagents/`` (panorama's, without its meta file)."""
        sub = self.dir / session / "subagents"
        sub.mkdir(parents=True, exist_ok=True)
        return Transcript(sub / f"agent-{agent_id}.jsonl", session, agent_id)
