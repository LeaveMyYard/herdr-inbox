"""``inbox`` command line. Heavy imports (mcp, textual) are lazy so the hook stays fast."""

from __future__ import annotations

import argparse
import logging
import sys


def _age(iso: str) -> str:
    from datetime import datetime, timezone

    secs = int((datetime.now(timezone.utc) - datetime.fromisoformat(iso)).total_seconds())
    for unit, size in (("d", 86400), ("h", 3600), ("m", 60)):
        if secs >= size:
            return f"{secs // size}{unit}"
    return f"{max(secs, 0)}s"


RADAR_MESSAGES = {
    "changed": "$inbox_q {verb} herdr-radar's sidebar rows in {path} (backup: config.toml.inbox-bak); herdr reloaded",
    "unchanged": "nothing to change: herdr-radar's sidebar rows in {path} are already as asked",
    "no-config": "no herdr config at {path}",
    "no-radar": "herdr-radar is not installed (or disabled): nothing to do",
    "no-block": "no herdr-radar sidebar block in {path}: nothing to do",
    "invalid": "{path} does not parse as TOML: left untouched",
    "stepped-aside": "herdr-radar already carries $inbox_q (extra_tokens): left to radar",
    "unsupported": "herdr-radar's block in {path} uses TOML this command does not edit ([[tables]] or multi-line strings): left untouched",
    "conflict": "{path} changed while editing: left untouched, run it again",
}


def main(argv: list[str] | None = None, stdin: str | None = None) -> int:
    parser = argparse.ArgumentParser(prog="inbox", description="Non-blocking operator question inbox for herdr")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("mcp", help="run the MCP stdio server")
    sub.add_parser("tui", help="run the inbox TUI (herdr popup)")
    p = sub.add_parser("open", help="ask herdr to open the inbox popup (optionally on one question or group)")
    target = p.add_mutually_exclusive_group()
    target.add_argument("--question", metavar="Qn", help="open on this question")
    target.add_argument("--group", metavar="G", help="open on this group")
    target.add_argument("--pane", metavar="PANE_ID", help="open on this pane's oldest open group")
    target.add_argument("--session", metavar="SID", help="open on this Claude session's oldest open group")
    p.add_argument("--no-hold", action="store_true",
                   help="open like a normal popup (0.4 s arming, then Enter works) instead of held until Tab/→; "
                        "pass it only when an explicit operator key or click opened the inbox")
    p = sub.add_parser("list", help="print questions")
    p.add_argument("--all", action="store_true", help="include answered and closed questions")
    p = sub.add_parser("answer", help="answer a question and deliver it")
    p.add_argument("id", type=lambda s: int(s.lstrip("Qq")))
    p.add_argument("text", nargs="+")
    p = sub.add_parser("dismiss", help="close a question without answering")
    p.add_argument("id", type=lambda s: int(s.lstrip("Qq")))
    p = sub.add_parser("redeliver", help="deliver answered questions again (and make them unseen for check_answers)")
    p.add_argument("ids", nargs="+", type=lambda s: int(s.lstrip("Qq")))
    p = sub.add_parser("sync-tokens", help="recompute sidebar tokens and tab-label counts")
    p.add_argument("--remove-prefixes", action="store_true", help="strip every tab prefix (before uninstalling)")
    p = sub.add_parser("radar-sidebar", help="show the ❓N count ($inbox_q) in herdr-radar's sidebar rows")
    p.add_argument("--remove", action="store_true", help="take it out again (before uninstalling)")
    p = sub.add_parser("clean-venvs", help="remove old versions' MCP venvs, including ones from before 0.5.3")
    p.add_argument("--data", help="the plugin data dir (default: $CLAUDE_PLUGIN_DATA or ~/.claude/plugins/data/herdr-inbox-herdr-inbox)")
    p = sub.add_parser("deliver", help="deliver answered questions now (the popup's detached sender)")
    p.add_argument("ids", nargs="+", type=lambda s: int(s.lstrip("Qq")))
    p.add_argument("--force", action="store_true", help="the operator's Send now: skip the busy grace and focus wait")
    p = sub.add_parser("verify", help="(internal) check a typed message landed and record it under its claims")
    p.add_argument("job")
    p = sub.add_parser("worker", help="run the delivery worker (one per machine; retries every pending answer)")
    p.add_argument("--ensure", action="store_true", help="start it in the background unless it is running, then return")
    sub.add_parser("export", help="(remote machines) questions as JSON; request on stdin")
    sub.add_parser("apply", help="(remote machines) one operator action; request on stdin")
    p = sub.add_parser("remote", help="remote herdr machines (docs/REMOTE.md)")
    rsub = p.add_subparsers(dest="remote_cmd", required=True)
    for name, text in (("install", "install both plugins on a saved herdr machine over ssh"),
                       ("upgrade", "upgrade both plugins on a saved herdr machine over ssh")):
        rp = rsub.add_parser(name, help=text)
        rp.add_argument("machine", help="label or id from `herdr machine list`")
        rp.add_argument("--ref", help="git branch/tag to install (default: the default branch)")
    p = sub.add_parser("hook", help="Claude Code hook entry point")
    p.add_argument("event", choices=["session-start", "subagent-start", "post-tool-batch", "stop", "rewake-watch"])
    args = parser.parse_args(argv)

    if args.cmd == "hook":
        from .hook import run

        return run(args.event)
    if args.cmd == "mcp":
        from . import venvs

        venvs.claim_current()  # before any heavy import: this venv is in use (venvs.py)
        from .server import run

        run()
        return 0
    if args.cmd == "tui":
        from .tui import InboxApp

        InboxApp().run()
        return 0

    logging.basicConfig(level=logging.WARNING, format="inbox: %(message)s")
    from . import herdr, service
    from .store import Store

    if args.cmd == "radar-sidebar":
        from . import radar

        status = radar.ensure(remove=args.remove)
        print(RADAR_MESSAGES[status].format(path=radar.config_path(), verb="removed from" if args.remove else "added to"))
        return 0 if status in ("changed", "unchanged", "stepped-aside") else 1
    if args.cmd == "clean-venvs":
        import os
        from importlib.metadata import version
        from pathlib import Path

        from . import venvs

        data = Path(args.data or os.environ.get("CLAUDE_PLUGIN_DATA")
                    or Path.home() / ".claude" / "plugins" / "data" / "herdr-inbox-herdr-inbox")
        removed = venvs.cleanup(data, version("herdr-inbox"), lockless=True)
        for path in removed:
            print(f"removed {path}")
        if not removed:
            print(f"nothing to remove in {data}")
        return 0
    if args.cmd == "deliver":
        with Store() as store:
            qs = [q for q in (store.get(i) for i in args.ids)
                  if q and q.status == "answered" and not q.delivered_at and not q.acknowledged_at]
            service._ensure_worker()
            if qs:
                # after the popup closed: keep out of the pane the operator is in (the worker's
                # rule) unless this is Send now
                service.deliver_batch(store, qs, retry=not args.force, force=args.force)
        return 0
    if args.cmd == "verify":
        import json as _json

        with Store() as store:
            service.verify_job(store, _json.loads(args.job))
        return 0
    if args.cmd == "worker":
        from . import worker

        if args.ensure:
            worker.ensure()
            return 0
        return worker.run()
    if args.cmd in ("export", "apply"):
        import json as _json

        from . import remote_api

        code, out = remote_api.run_cli(args.cmd, sys.stdin.read() if stdin is None else stdin)
        print(_json.dumps(out))
        return code
    if args.cmd == "remote":
        from . import remote

        return remote.install(args.machine, upgrade=args.remote_cmd == "upgrade", ref=args.ref)
    if args.cmd == "open":
        from . import deeplink

        env = {}
        for kind in deeplink.KINDS:
            if getattr(args, kind):
                value = getattr(args, kind)
                if not deeplink.valid(kind, value):
                    parser.error(f"--{kind} {value!r}: expected " + ("Qn or n" if kind == "question" else
                                                                       "a group number" if kind == "group" else "an id"))
                if deeplink.popup_is_running():
                    print("the inbox popup is already open; close it and run this again", file=sys.stderr)
                    return 3
                env[deeplink.NONCE_ENV] = deeplink.write_request(
                    kind, value.lstrip("Qq") if kind == "question" else value, hold=not args.no_hold)
        err = herdr.open_popup(env)
        if err:
            print(f"could not open the inbox popup: {err}", file=sys.stderr)
            return 1
        return 0

    store = Store()
    if args.cmd == "list":
        for q in store.list(include_closed=args.all):
            where = q.tab_label or q.pane_id or "-"
            by = f" [{q.asked_by}]" if q.asked_by else ""
            rep = f" x{q.repeat_count + 1}" if q.repeat_count else ""
            print(f"{q.ref:>5} {q.status:<9} {_age(q.created_at):>4} {where}{by}{rep}: {service.first_line(q.question, 100)}")
            if args.all and q.answer:
                print(f"      -> {q.answer}")
        return 0
    if args.cmd in ("answer", "redeliver"):
        from . import worker

        worker.ensure()  # it retries whatever this cannot deliver right away
    if args.cmd == "answer":
        q = service.answer(store, args.id, " ".join(args.text))
        if q is None:
            print(f"Q{args.id} is not open", file=sys.stderr)
            return 1
        state = f"delivered to {q.pane_id}" if q.delivered_at else f"not delivered: {q.delivery_error}"
        print(f"{q.ref} answered, {state}")
        return 0
    if args.cmd == "redeliver":
        done = store.redeliver(args.ids)
        missing = sorted(set(args.ids) - set(done))
        if done:
            service.deliver_batch(store, [store.get(i) for i in done])
            for qid in done:
                q = store.get(qid)
                state = "delivered (verified)" if q.verified_at else (q.delivery_error or "sent, not verified")
                print(f"Q{qid}: {state}; available to check_answers until seen")
        for qid in missing:
            print(f"Q{qid}: not an answered question", file=sys.stderr)
        return 1 if missing else 0
    if args.cmd == "dismiss":
        if not service.close(store, args.id, "dismissed"):
            print(f"Q{args.id} is not open", file=sys.stderr)
            return 1
        print(f"Q{args.id} dismissed")
        return 0
    if args.cmd == "sync-tokens":
        service.sync_tokens(store, remove_prefixes=args.remove_prefixes)
        return 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
