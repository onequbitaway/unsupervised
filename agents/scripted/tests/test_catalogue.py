"""The catalogue view: which verbs exist and how the agent fills their arguments."""

import unittest

import _path  # noqa: F401
from town import ITEMS, Town

from scripted_agent.catalogue import Catalogue
from unsupervised.testing import FakeWorld


def catalogue_with(verb: str, properties: dict, required: list) -> Catalogue:
    schema = {"type": "object", "properties": properties, "required": required}
    info = {"verb": verb, "module": "test", "summary": "A test verb.", "free": False, "scope": "zone", "args_schema": schema}
    return Catalogue({"verbs": [info], "items": ITEMS, "attrs": [], "conditions": []})


class ArgsTest(unittest.TestCase):
    def test_arguments_follow_the_names_and_types_of_the_schema(self) -> None:
        cases = [
            # (properties, required, values, expected arguments)
            ({"item": {"type": "string"}}, ["item"], {"item": "bread", "qty": 2}, {"item": "bread"}),
            ({"good": {"type": "string"}, "quantity": {"type": "integer"}}, ["good", "quantity"], {"item": "bread", "qty": 2}, {"good": "bread", "quantity": 2}),
            # A verb that takes the id of the item gets the id from the catalogue.
            ({"item": {"type": "integer"}, "qty": {"type": "integer"}}, ["item"], {"item": "bread", "qty": 3}, {"item": 1, "qty": 3}),
            ({"x": {"type": "integer"}, "y": {"type": "integer"}}, ["x", "y"], {"x": 4, "y": 9}, {"x": 4, "y": 9}),
            ({"to": {"type": "object"}}, ["to"], {"x": 4, "y": 9}, {"to": {"x": 4, "y": 9}}),
            ({"hours": {"type": ["integer", "null"]}}, [], {"hours": 8, "ticks": 480}, {"hours": 8}),
            ({"job_id": {"type": "integer"}, "note": {"type": "string"}}, ["job_id"], {"job": 5}, {"job_id": 5}),
            ({}, [], {"ticks": 5}, {}),
        ]
        for properties, required, values, expected in cases:
            with self.subTest(properties=properties):
                self.assertEqual(catalogue_with("do", properties, required).args_for("do", values), expected)

    def test_a_verb_is_left_alone_when_a_required_argument_has_no_value(self) -> None:
        cases = [
            # A name that the agent does not know.
            ({"ritual": {"type": "string"}}, ["ritual"], {"item": "bread"}),
            # A known name without a value.
            ({"job": {"type": "integer"}}, ["job"], {"item": "bread"}),
            # A value of the wrong type.
            ({"job": {"type": "integer"}}, ["job"], {"job": "street cleaner"}),
            # An item that the catalogue does not have, for a verb that wants its id.
            ({"item": {"type": "integer"}}, ["item"], {"item": "caviar"}),
            # A required name without a schema.
            ({}, ["secret"], {}),
        ]
        for properties, required, values in cases:
            with self.subTest(properties=properties):
                self.assertIsNone(catalogue_with("do", properties, required).args_for("do", values))

    def test_a_verb_that_does_not_exist_has_no_arguments(self) -> None:
        catalogue = Catalogue(FakeWorld().catalogue())
        self.assertIsNone(catalogue.args_for("eat", {"item": "bread"}))
        self.assertEqual(catalogue.args_for("move_to", {"x": 1, "y": 2}), {"x": 1, "y": 2})
        self.assertEqual(catalogue.first("sleep", "rest", "wait"), "wait")
        self.assertIsNone(catalogue.first("sleep", "rest"))


class CatalogueTest(unittest.TestCase):
    def setUp(self) -> None:
        self.catalogue = Catalogue(Town().world.catalogue())

    def test_food_is_what_has_the_tag_and_nutrition_and_the_best_comes_first(self) -> None:
        self.assertEqual([item["name"] for item in self.catalogue.food()], ["bread", "apple"])

    def test_the_view_knows_attributes_and_blocking_conditions(self) -> None:
        self.assertEqual(self.catalogue.attrs, {"need.food", "need.energy", "need.health"})
        self.assertEqual(self.catalogue.blocking, {"frozen", "jailed"})

    def test_a_verb_that_the_agent_gave_up_on_is_not_used(self) -> None:
        self.assertTrue(self.catalogue.has("eat"))
        self.assertEqual(self.catalogue.args_for("eat", {"item": "bread"}), {"item": "bread"})
        self.catalogue.give_up("eat", "invalid_args: the server refused the arguments")
        self.assertFalse(self.catalogue.has("eat"))
        self.assertIsNone(self.catalogue.args_for("eat", {"item": "bread"}))
        self.assertIsNone(self.catalogue.first("eat"))


if __name__ == "__main__":
    unittest.main()
