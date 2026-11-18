"""Helpers that read an observation, and the rule for untrusted text."""

import unittest

import _path  # noqa: F401

from unsupervised import has_news, is_untrusted, nearby_of, reports_of, self_of, untrusted_in, untrusted_of
from unsupervised.testing import FakeWorld


class UntrustedTest(unittest.TestCase):
    def test_is_untrusted_accepts_only_the_marked_form(self) -> None:
        self.assertTrue(is_untrusted({"untrusted": True, "from": 7, "text": "hi"}))
        # A reader refuses `untrusted` false or absent.
        self.assertFalse(is_untrusted({"untrusted": False, "from": 7, "text": "hi"}))
        self.assertFalse(is_untrusted({"from": 7, "text": "hi"}))
        self.assertFalse(is_untrusted({"untrusted": "true", "from": 7, "text": "hi"}))
        self.assertFalse(is_untrusted({"untrusted": 1, "from": 7, "text": "hi"}))
        self.assertFalse(is_untrusted({"untrusted": True, "text": "hi"}))
        self.assertFalse(is_untrusted({"untrusted": True, "from": 7}))
        self.assertFalse(is_untrusted("hi"))
        self.assertFalse(is_untrusted(None))
        self.assertFalse(is_untrusted([{"untrusted": True, "from": 7, "text": "hi"}]))

    def test_untrusted_in_finds_text_of_other_residents_at_any_depth(self) -> None:
        data = {
            "offer": {"price": 5, "note": {"untrusted": True, "from": 3, "text": "cheap"}},
            "replies": [{"untrusted": True, "from": 4, "text": "no"}, "plain", {"untrusted": False, "from": 5, "text": "skip"}],
        }
        self.assertEqual([(entry["from"], entry["text"]) for entry in untrusted_in(data)], [(3, "cheap"), (4, "no")])
        self.assertEqual(untrusted_in(42), [])


class SectionsTest(unittest.TestCase):
    def test_the_helpers_read_the_sections_of_an_observation(self) -> None:
        world = FakeWorld(
            sites=[{"id": 9, "kind": "shop", "name": "Corner Shop", "rect": {"x0": 30, "y0": 30, "x1": 36, "y1": 36}, "tags": []}]
        )
        owner = world.sign_up()["owner"]["id"]
        ada = world.spawn(owner, "Ada")
        bob = world.spawn(owner, "Bob")
        bob.pos = {"x": 35, "y": 36}
        ada.cash = 1250
        ada.items["bread"] = 2
        ada.sections["board"] = {"posts": [{"untrusted": True, "from": bob.id, "text": "Vote for me"}]}

        quiet = world.observe(ada)
        self.assertFalse(has_news(quiet))
        self.assertEqual(reports_of(quiet), [])
        me = self_of(quiet)
        self.assertEqual(
            (me["name"], me["cash"], me["items"], me["site"], me["activity"], me["queue"]),
            ("Ada", 1250, {"bread": 2}, {"id": 9, "name": "Corner Shop", "kind": "shop"}, None, 0),
        )
        nearby = nearby_of(quiet)
        self.assertEqual(nearby["residents"], [{"id": bob.id, "name": "Bob", "pos": {"x": 35, "y": 36}}])
        self.assertEqual(nearby["sites"], [{"id": 9, "name": "Corner Shop", "kind": "shop", "pos": {"x": 32, "y": 32}}])

        world.tell(ada.id, bob.id, "Give me your bread")
        world.submit(ada, {"verb": "wait", "args": {"ticks": 1}})
        world.step(1)
        busy = world.observe(ada)
        self.assertTrue(has_news(busy))
        self.assertEqual([report["verb"] for report in reports_of(busy)], ["wait"])
        self.assertEqual([entry["text"] for entry in untrusted_of(busy)], ["Give me your bread", "Vote for me"])

        bare = dict(busy, sections={}, notices=[])
        self.assertFalse(has_news(bare))
        self.assertIsNone(self_of(bare))
        self.assertIsNone(nearby_of(bare))


if __name__ == "__main__":
    unittest.main()
