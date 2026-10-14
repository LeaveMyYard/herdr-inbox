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


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="inbox", description="Non-blocking operator question inbox for herdr")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("mcp", help="run the MCP stdio server")
    sub.add_parser("tui", help="run the inbox TUI (herdr popup)")
    sub.add_parser("open", help="ask herdr to open the inbox popup")
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
    p = sub.add_parser("hook", help="Claude Code hook entry point")
    p.add_argument("event", choices=["session-start", "subagent-start"])
    args = parser.parse_args(argv)

    if args.cmd == "hook":
        from .hook import run

        return run(args.event)
    if args.cmd == "mcp":
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

    if args.cmd == "open":
        err = herdr.open_popup()
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
