"""Named checks: print each, remember failures, exit non-zero if any failed."""

from __future__ import annotations

import sys


class Skip(Exception):
    """Raised by a probe that cannot run here (a resource this machine lacks).
    The launcher prints `SKIP <name> (<reason>)`; the run does not fail."""

    def __init__(self, reason: str) -> None:
        super().__init__(reason)
        self.reason = reason


class Verdict:
    def __init__(self, name: str) -> None:
        self.name = name
        self.failed: list[str] = []
        self.count = 0

    def check(self, label: str, ok: bool, detail: str = "") -> bool:
        self.count += 1
        print(f"  [{'ok' if ok else 'FAIL'}] {label}" + ("" if ok or not detail else f"\n         {detail}"))
        if not ok:
            self.failed.append(label)
        return bool(ok)

    def finish(self) -> int:
        print(f"{self.name}: {self.count - len(self.failed)}/{self.count} checks passed")
        return 1 if self.failed else 0

    def exit(self) -> None:
        sys.exit(self.finish())
