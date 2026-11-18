"""The library example of README.md must run."""

import contextlib
import io
import re
import unittest
import urllib.request
from typing import List
from unittest import mock

import _path
from town import FOOD, HEALTH, Town

from scripted_agent import Agent, State
from scripted_agent.cli import parse_args
from unsupervised.testing import DirectOpener, FakeGateway, direct_client

README = _path.PACKAGE_ROOT / "README.md"
#: The address that the example uses for the server.
EXAMPLE_URL = "https://unsupervised.example"


def timing_of(town: Town) -> dict:
    """Returns the clock of an agent that lives in the time of a town: it does not sleep."""
    return {"sleep": lambda seconds: None, "clock": lambda: float(town.world.tick)}


def python_blocks() -> List[str]:
    """Returns the content of every `python` block of the README, in order."""
    return re.findall(r"^```python\n(.*?)^```$", README.read_text(encoding="utf-8"), flags=re.MULTILINE | re.DOTALL)


class ReadmeTest(unittest.TestCase):
    def test_the_library_example_joins_and_keeps_the_resident_alive(self) -> None:
        (example,) = python_blocks()
        self.assertIn(EXAMPLE_URL, example)
        # "A ten-line example": the code without empty lines.
        self.assertLessEqual(len([line for line in example.splitlines() if line.strip()]), 10)

        town = Town()
        gateway = FakeGateway(town.world)
        sleeps: List[float] = []
        output = io.StringIO()
        # The example makes its own client and its own agent. The test gives
        # the requests of the client to the fake gateway, without a network.
        # It gives the agent the tick of the world as its clock, so the agent
        # does not sleep.
        timing = {"sleep": sleeps.append, "clock": lambda: float(town.world.tick)}
        with mock.patch.object(urllib.request, "urlopen", DirectOpener(gateway).open), mock.patch.dict(
            Agent.__init__.__kwdefaults__, timing
        ), contextlib.redirect_stdout(output):
            exec(compile(example, str(README), "exec"), {"__name__": "__example__"})
        # Every request of the example went to the address that the README shows.
        self.assertGreater(len(gateway.requests), 50)

        resident = town.world.residents[1]
        self.assertEqual((resident.name, resident.model), ("Ada", "scripted"))
        printed = output.getvalue().splitlines()
        self.assertEqual(printed[0], "store this key: %s" % resident.key)
        state, _, routines = printed[1].partition(" ")
        self.assertIn(state, {one.value for one in State})
        expected = ["eat meal when hungry", "eat bread when hungry", "eat vegetables when hungry", "sleep when exhausted"]
        self.assertEqual(routines, str(expected))
        self.assertEqual(len(printed), 2)
        # Fifty decisions are some days of game time. The resident worked, ate and stayed healthy.
        self.assertGreater(town.world.tick, 1440)
        self.assertGreaterEqual(town.shifts[resident.id], 2)
        self.assertGreaterEqual(town.meals[resident.id], 1)
        self.assertGreater(town.lowest[resident.id][FOOD], 300)
        self.assertEqual(town.lowest[resident.id][HEALTH], 1000)
        # The configuration of the example reached the agent: it eats below 700.
        # An agent with the default (600) lets the food fall lower in the same town.
        other = Town()
        default = Agent(direct_client(FakeGateway(other.world), other.spawn().key), **timing_of(other))
        default.run(decisions=50)
        self.assertGreater(town.lowest[resident.id][FOOD], other.lowest[1][FOOD])
        self.assertEqual([delay for delay in sleeps if delay > 2.0], [])

    def test_the_command_of_the_readme_is_a_valid_command_line(self) -> None:
        text = README.read_text(encoding="utf-8").replace("\\\n", " ")
        commands = re.findall(r"python3 -m scripted_agent\s+(--.*)$", text, flags=re.MULTILINE)
        self.assertEqual(len(commands), 2)
        args = parse_args(commands[0].split())
        self.assertEqual((args.url, args.residents, args.verbose), ("http://127.0.0.1:8080", 5, True))
        # The command of the house residents.
        house = parse_args(commands[1].split())
        self.assertEqual((house.url, house.residents, house.model), ("http://127.0.0.1:8080", 60, "house"))
        self.assertEqual(house.start_spread, 240)

    def test_the_option_table_names_every_option_of_the_command(self) -> None:
        text = README.read_text(encoding="utf-8")
        documented = re.findall(r"^\| `(--[a-z-]+)` \|", text, flags=re.MULTILINE)
        with contextlib.redirect_stdout(io.StringIO()) as usage, self.assertRaises(SystemExit):
            parse_args(["--help"])
        real = [option for option in re.findall(r"(--[a-z-]+)", usage.getvalue()) if option != "--help"]
        self.assertEqual(sorted(documented), sorted(set(real)))


if __name__ == "__main__":
    unittest.main()
