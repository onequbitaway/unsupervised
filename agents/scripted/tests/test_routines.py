"""Routines follow the catalogue: only what exists gets a routine."""

import unittest

import _path  # noqa: F401
from town import Town, kernel_verbs

from scripted_agent import AgentConfig, Catalogue, build_routines
from unsupervised.testing import FakeWorld


def names(routines: list) -> list:
    return [routine["name"] for routine in routines]


class RoutinesTest(unittest.TestCase):
    def test_a_world_with_only_the_kernel_verbs_gets_no_routine(self) -> None:
        catalogue = Catalogue(FakeWorld(verbs=kernel_verbs()).catalogue())
        self.assertEqual(build_routines(catalogue, AgentConfig()), [])

    def test_the_full_town_gets_eat_sleep_and_work_routines_in_priority_order(self) -> None:
        town = Town()
        routines = build_routines(Catalogue(town.world.catalogue()), AgentConfig())
        self.assertEqual(names(routines), ["eat bread when hungry", "eat apple when hungry", "sleep when tired", "work in the day"])
        self.assertEqual(
            routines[0],
            {
                "name": "eat bread when hungry",
                "when": {
                    "type": "all",
                    "of": [
                        {"type": "attr_below", "attr": "need.food", "value": 500},
                        {"type": "has_item", "item": "bread", "qty": 1},
                    ],
                },
                "then": {"verb": "eat", "args": {"item": "bread"}},
            },
        )
        self.assertEqual(routines[2]["then"], {"verb": "sleep", "args": {}})
        self.assertEqual(
            routines[3]["when"],
            {
                "type": "all",
                "of": [
                    {"type": "clock_between", "from_hour": 8, "to_hour": 18},
                    {"type": "attr_above", "attr": "need.energy", "value": 300},
                ],
            },
        )
        # The server takes the list: every verb, attribute and item in it exists.
        self.assertEqual(town.world.set_routines(town.spawn(), routines), {"routines": 4})

    def test_the_numbers_come_from_the_configuration(self) -> None:
        config = AgentConfig(eat_below=350, work_from=6, work_to=14, sleep_below=100)
        routines = build_routines(Catalogue(Town().world.catalogue()), config)
        self.assertEqual(routines[0]["when"]["of"][0]["value"], 350)
        self.assertEqual(routines[2]["when"]["of"][0], {"type": "attr_below", "attr": "need.energy", "value": 100})
        self.assertEqual(routines[3]["when"]["of"][0], {"type": "clock_between", "from_hour": 6, "to_hour": 14})

    def test_a_verb_that_is_missing_gets_no_routine(self) -> None:
        town = Town(verbs=["eat", "work"])
        self.assertEqual(
            names(build_routines(Catalogue(town.world.catalogue()), AgentConfig())),
            ["eat bread when hungry", "eat apple when hungry", "work in the day"],
        )
        only_sleep = Town(verbs=["sleep"])
        self.assertEqual(names(build_routines(Catalogue(only_sleep.world.catalogue()), AgentConfig())), ["sleep when tired"])

    def test_an_attribute_that_is_missing_leaves_out_what_needs_it(self) -> None:
        raw = Town().world.catalogue()
        raw["attrs"] = [attr for attr in raw["attrs"] if attr["name"] != "need.energy"]
        routines = build_routines(Catalogue(raw), AgentConfig())
        self.assertEqual(names(routines), ["eat bread when hungry", "eat apple when hungry", "work in the day"])
        # Without the energy attribute, the work routine has only the clock.
        self.assertEqual(routines[2]["when"], {"type": "all", "of": [{"type": "clock_between", "from_hour": 8, "to_hour": 18}]})

    def test_a_verb_with_arguments_that_the_agent_cannot_fill_gets_no_routine(self) -> None:
        raw = Town().world.catalogue()
        for verb in raw["verbs"]:
            if verb["verb"] == "work":
                verb["args_schema"] = {"type": "object", "properties": {"contract": {"type": "integer"}}, "required": ["contract"]}
        self.assertNotIn("work in the day", names(build_routines(Catalogue(raw), AgentConfig())))


if __name__ == "__main__":
    unittest.main()
