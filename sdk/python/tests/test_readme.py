"""The examples of README.md must run."""

import contextlib
import io
import re
import unittest
import urllib.request
from typing import List
from unittest import mock

import _path

from unsupervised.testing import FakeGateway, FakeServer, FakeWorld

README = _path.PACKAGE_ROOT / "README.md"
#: The address that the examples use for the server.
EXAMPLE_URL = "https://unsupervised.example"


def python_blocks() -> List[str]:
    """Returns the content of every `python` block of the README, in order."""
    return re.findall(r"^```python\n(.*?)^```$", README.read_text(encoding="utf-8"), flags=re.MULTILINE | re.DOTALL)


def run(code: str) -> List[str]:
    """Runs example code and returns the lines that it printed."""
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        exec(compile(code, str(README), "exec"), {"__name__": "__example__"})
    return output.getvalue().splitlines()


class ReadmeTest(unittest.TestCase):
    def test_the_first_example_joins_acts_and_waits(self) -> None:
        example = python_blocks()[0]
        self.assertIn(EXAMPLE_URL, example)
        # "A ten-line example": the code without empty lines.
        self.assertLessEqual(len([line for line in example.splitlines() if line.strip()]), 10)

        world = FakeWorld(time="auto")
        server = FakeServer(FakeGateway(world))
        self.addCleanup(server.close)
        # The example makes its own client. Send its requests to the fake server without a proxy.
        with mock.patch.object(urllib.request, "urlopen", server.opener.open):
            printed = run(example.replace(EXAMPLE_URL, server.url))

        resident = world.residents[1]
        self.assertEqual((resident.name, resident.model), ("Ada", "my-model"))
        self.assertEqual(
            printed,
            [
                "store this key: %s" % resident.key,
                "cancel, set_routines, wait, move_to",
                "started {'x': 32, 'y': 32}",
                "move_to True {'x': 34, 'y': 32}",
            ],
        )
        # The wait of the example ended with the report of the walk: 2 ticks, not 120.
        self.assertEqual(world.tick, 2)

    def test_the_test_support_example_runs(self) -> None:
        examples = [block for block in python_blocks() if "FakeServer" in block]
        self.assertEqual(len(examples), 1)
        with mock.patch.object(FakeWorld, "step", autospec=True, side_effect=FakeWorld.step) as step:
            self.assertEqual(run(examples[0]), [])
        # The example ran to its last line.
        self.assertEqual([call.args[1] for call in step.call_args_list], [5])


if __name__ == "__main__":
    unittest.main()
