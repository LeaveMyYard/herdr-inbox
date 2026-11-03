"""Rewrite the `scenarios! { … }` list in crates/inbox-core/tests/popup_transitions.rs from
tests/fixtures/popup-transitions.json: one Rust test per scenario (`<module> { <fn> = "<name>", }`).
Run it after tools/capture_popup_transitions.py adds or renames a scenario."""

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TEST = ROOT / "crates" / "inbox-core" / "tests" / "popup_transitions.rs"


def main() -> None:
    golden = json.loads((ROOT / "tests" / "fixtures" / "popup-transitions.json").read_text())
    modules: dict[str, list[tuple[str, str]]] = {}
    for scenario in golden["scenarios"]:
        module, rest = scenario["name"].split(".", 1)
        modules.setdefault(module, []).append((rest.replace(".", "_"), scenario["name"]))
    body = "".join(f"    {m} {{\n" + "".join(f'        {fn} = "{name}",\n' for fn, name in items) + "    }\n"
                   for m, items in modules.items())
    text = TEST.read_text()
    new, n = re.subn(r"scenarios! \{\n.*?\n\}\n", "scenarios! {\n" + body + "}\n", text, count=1, flags=re.S)
    assert n == 1, "no scenarios! block"
    TEST.write_text(new)


if __name__ == "__main__":
    main()
