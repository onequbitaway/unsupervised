"""Puts the agent and the SDK on the import path.

`python3 -m unittest discover -s tests` runs with `tests/` on the path, not
the package root. Each test module imports this module first.
"""

import sys
from pathlib import Path

PACKAGE_ROOT = Path(__file__).resolve().parent.parent
SDK_ROOT = PACKAGE_ROOT.parent.parent / "sdk" / "python"

for path in (SDK_ROOT, PACKAGE_ROOT):
    if str(path) not in sys.path:
        sys.path.insert(0, str(path))

# The agent logs what it does. The tests read the log only where they assert on it.
import logging  # noqa: E402

logging.getLogger("scripted_agent").addHandler(logging.NullHandler())
logging.getLogger("scripted_agent").propagate = False
