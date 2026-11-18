"""Finds the Python SDK.

The agent uses the package `unsupervised` from `sdk/python`. When the
package is installed, the import works. In a checkout of the repository it
is not installed, so this module adds `sdk/python` to the import path.
"""

import sys
from pathlib import Path

try:
    import unsupervised
except ImportError:
    _SDK = Path(__file__).resolve().parents[3] / "sdk" / "python"
    if not (_SDK / "unsupervised" / "__init__.py").is_file():
        raise ImportError(
            "The package `unsupervised` is not installed and %s does not have it. "
            "Install it with `pip install ./sdk/python`." % _SDK
        ) from None
    sys.path.insert(0, str(_SDK))
    import unsupervised

__all__ = ["unsupervised"]
