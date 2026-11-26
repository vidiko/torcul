# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Check that security suppressions name rules and explain their use."""

# Run from the repository root: python3 -I -B docs/hooks/test_suppressions.py .

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve()
BANDIT = "no" + "sec"
SEMGREP = "no" + "semgrep"
MARKER = re.compile(r"#.*?\b(?:" + BANDIT + "|" + SEMGREP + r")\b")
RULES = re.compile(
    r"# (?:"
    + BANDIT
    + r" B\d{3}(?:, B\d{3})*|"
    + SEMGREP
    + r": [a-z0-9][a-z0-9.-]*(?:, [a-z0-9][a-z0-9.-]*)*)"
    + r"  # (\S+(?:\s+\S+)+)\s*$"
)


def valid(line: str) -> bool:
    """Accept unmarked lines or a complete suppression with a reason."""
    marker = MARKER.search(line)
    return marker is None or RULES.fullmatch(line[marker.start() :]) is not None


def main() -> int:
    """Run planted cases and check every hook and workflow source."""
    cases = [
        ("bandit", f"x  # {BANDIT} B404  # Fixed arguments.", True),
        ("bandit multiple", f"# {BANDIT} B404, B603  # Fixed arguments.", True),
        ("semgrep", f"# {SEMGREP}: rule.id  # Fixed arguments.", True),
        ("semgrep multiple", f"# {SEMGREP}: a, b-c  # Fixed arguments.", True),
        ("bare bandit", f"# {BANDIT}", False),
        ("bandit no reason", f"# {BANDIT} B603", False),
        ("bandit wrong separator", f"# {BANDIT} B603 -- two words", False),
        ("bare semgrep", f"# {SEMGREP}", False),
        ("semgrep no reason", f"# {SEMGREP}: x", False),
        ("uppercase id", f"# {SEMGREP}: X  # Two words", False),
        ("one word", f"# {BANDIT} B404  # Reason", False),
        ("two words", f"# {BANDIT} B404  # Two words", True),
        ("marker in reason", f"# {BANDIT} B404  # {SEMGREP} marker", True),
    ]
    bad = 0
    for name, line, expected in cases:
        ok = valid(line) == expected
        print("ok  " if ok else "FAIL", name)
        bad += not ok
    paths = sorted((ROOT / "docs/hooks").glob("*.py"))
    paths += sorted((ROOT / "tools").rglob("*.py"))
    paths += sorted((ROOT / ".github/workflows").glob("*.yml"))
    violations = [
        f"{path.relative_to(ROOT)}:{number}"
        for path in paths
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1)
        if not valid(line)
    ]
    print("ok  " if not violations else "FAIL", "real tree", ", ".join(violations))
    bad += bool(violations)
    print(f"cases={len(cases) + 1} failed={bad}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
