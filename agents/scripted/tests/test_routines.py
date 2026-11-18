"""Routines follow the catalogue: only what exists gets a routine."""

import unittest

import _path  # noqa: F401
from town import Town, kernel_verbs

from scripted_agent import AgentConfig, Catalogue, build_routines
from unsupervised.testing import FakeWorld

EAT = ["eat meal when hungry", "eat bread when hungry", "eat vegetables when hungry"]


def names(routines: list) -> list:
    return [routine["name"] for routine in routines]


class RoutinesTest(unittest.TestCase):
    def test_a_world_with_only_the_kernel_verbs_gets_no_routine(self) -> None:
        catalogue = Catalogue(FakeWorld(verbs=kernel_verbs()).catalogue())
        self.assertEqual(build_routines(catalogue, AgentConfig()), [])

    def test_the_full_town_gets_eat_and_sleep_routines_in_priority_order(self) -> None:
        town = Town()
        routines = build_routines(Catalogue(town.world.catalogue()), AgentConfig())
        self.assertEqual(names(routines), EAT + ["sleep when exhausted"])
        self.assertEqual(
            routines[0],
            {
                "name": "eat meal when hungry",
                "when": {
                    "type": "all",
                    "of": [
                        {"type": "attr_below", "attr": "need.food", "value": 300},
                        {"type": "has_item", "item": "meal", "qty": 1},
                    ],
                },
                "then": {"verb": "eat", "args": {"item": "meal"}},
            },
        )
        self.assertEqual(
            routines[3],
            {
                "name": "sleep when exhausted",
                "when": {"type": "attr_below", "attr": "need.energy", "value": 150},
                "then": {"verb": "sleep", "args": {}},
            },
        )
        # The server takes the list: every verb, attribute and item in it exists.
        self.assertEqual(town.world.set_routines(town.spawn(), routines), {"routines": 4})

    def test_the_numbers_come_from_the_configuration(self) -> None:
        config = AgentConfig(routine_eat_below=350, routine_sleep_below=100)
        routines = build_routines(Catalogue(Town().world.catalogue()), config)
        self.assertEqual(routines[0]["when"]["of"][0]["value"], 350)
        self.assertEqual(routines[3]["when"], {"type": "attr_below", "attr": "need.energy", "value": 100})

    def test_a_verb_that_is_missing_gets_no_routine(self) -> None:
        town = Town(verbs=["eat", "work"])
        self.assertEqual(names(build_routines(Catalogue(town.world.catalogue()), AgentConfig())), EAT)
        only_sleep = Town(verbs=["sleep"])
        self.assertEqual(
            names(build_routines(Catalogue(only_sleep.world.catalogue()), AgentConfig())), ["sleep when exhausted"]
        )

    def test_an_attribute_that_is_missing_leaves_out_what_needs_it(self) -> None:
        raw = Town().world.catalogue()
        raw["attrs"] = [attr for attr in raw["attrs"] if attr["name"] != "need.energy"]
        self.assertEqual(names(build_routines(Catalogue(raw), AgentConfig())), EAT)

    def test_a_verb_with_arguments_that_the_agent_cannot_fill_gets_no_routine(self) -> None:
        raw = Town().world.catalogue()
        for verb in raw["verbs"]:
            if verb["verb"] == "eat":
                verb["args_schema"] = {"type": "object", "properties": {"ritual": {"type": "string"}}, "required": ["ritual"]}
        self.assertEqual(names(build_routines(Catalogue(raw), AgentConfig())), ["sleep when exhausted"])


if __name__ == "__main__":
    unittest.main()
