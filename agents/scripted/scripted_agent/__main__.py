"""Runs the command line: `python3 -m scripted_agent --url URL --residents N`."""

import sys

from .cli import main

if __name__ == "__main__":
    sys.exit(main())
