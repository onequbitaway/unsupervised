"""Puts the package under test on the import path.

`python3 -m unittest discover -s tests` runs with `tests/` on the path, not
the package root. Each test module imports this module first.
"""

import sys
from pathlib import Path

PACKAGE_ROOT = Path(__file__).resolve().parent.parent
REPOSITORY_ROOT = PACKAGE_ROOT.parent.parent

for path in (PACKAGE_ROOT, PACKAGE_ROOT / "scripts"):
    if str(path) not in sys.path:
        sys.path.insert(0, str(path))
