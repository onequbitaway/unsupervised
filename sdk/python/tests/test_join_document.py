"""The examples of `docs/join.md` must work.

An agent copies the examples of the join document. This module runs every
`curl` example against the fake server, in the order of the document, and
compares the example observation with the schema and with the fake world.
"""

import json
import re
import shlex
import unittest
from typing import Any, Dict, List, NamedTuple, Optional

import _path

from unsupervised.testing import Busy, FakeGateway, FakeVerb, FakeWorld, verb_info

JOIN = _path.REPOSITORY_ROOT / "docs" / "join.md"
SCHEMA = _path.REPOSITORY_ROOT / "schema" / "protocol.schema.json"

#: The place holders of the document for the two secrets.
OWNER_TOKEN = "OWNER_TOKEN"
RESIDENT_KEY = "RESIDENT_KEY"


class Curl(NamedTuple):
    """One `curl` example of the document."""

    method: str
    #: Path and query, without the address of the server.
    url: str
    headers: Dict[str, str]
    body: str

    @property
    def route(self) -> str:
        return "%s %s" % (self.method, self.url.split("?")[0])


def blocks(language: str) -> List[str]:
    """Returns the content of every fenced block of one language, in order."""
    text = JOIN.read_text(encoding="utf-8")
    return re.findall(r"^```%s\n(.*?)^```$" % language, text, flags=re.MULTILINE | re.DOTALL)


def curls() -> List[Curl]:
    """Reads every `curl` command of the `sh` blocks."""
    found = []
    for block in blocks("sh"):
        # A backslash at the end of a line joins it with the next line.
        words = shlex.split(block.replace("\\\n", " "), comments=True)
        starts = [index for index, word in enumerate(words) if word == "curl"]
        for start, end in zip(starts, starts[1:] + [len(words)]):
            found.append(_curl(words[start + 1 : end]))
    return found


def _curl(words: List[str]) -> Curl:
    method: Optional[str] = None
    url = ""
    headers: Dict[str, str] = {}
    body = ""
    index = 0
    while index < len(words):
        word = words[index]
        if word == "-s":
            index += 1
        elif word in ("-X", "-H", "-d"):
            value = words[index + 1]
            if word == "-X":
                method = value
            elif word == "-d":
                body = value
            else:
                name, _, content = value.partition(":")
                headers[name.strip().lower()] = content.strip()
            index += 2
        elif word.startswith("$BASE/"):
            url = word[len("$BASE") :]
            index += 1
        else:
            raise AssertionError("the test does not know the curl word %r" % word)
    return Curl(method or ("POST" if body else "GET"), url, headers, body)


def town(spawn: Optional[Dict[str, int]] = None) -> FakeWorld:
    """A fake world that has what the examples name: the verbs `eat` and `sleep`, a need and an item."""
    world = FakeWorld(
        time="auto",
        spawn=spawn,
        attrs=[{"name": "need.food", "min": 0, "max": 1000, "default": 1000, "summary": "0 means starving."}],
        items=[{"id": 0, "name": "bread", "weight": 1, "tags": ["food"], "attrs": {"nutrition": 300}, "summary": "A loaf."}],
        sites=[{"id": 9, "kind": "shop", "name": "Corner Shop", "rect": {"x0": 30, "y0": 30, "x1": 36, "y1": 36}, "tags": []}],
    )
    item = {"type": "string", "description": "Name of the item."}
    world.add_verb(FakeVerb(verb_info("eat", "life", "Eat.", properties={"item": item}, required=["item"]), lambda *_: Busy(5)))
    world.add_verb(FakeVerb(verb_info("sleep", "life", "Sleep."), lambda *_: Busy(60)))
    return world


class CurlExamplesTest(unittest.TestCase):
    def setUp(self) -> None:
        # A new resident stands 12 tiles from the target of the first `move_to` example.
        self.world = town(spawn={"x": 118, "y": 60})
        self.gateway = FakeGateway(self.world)
        self.secrets: Dict[str, str] = {}

    def send(self, curl: Curl) -> Any:
        """Sends one example with the real secrets in the place of the place holders. Returns the JSON answer."""
        headers = dict(curl.headers)
        for place_holder, secret in self.secrets.items():
            if "authorization" in headers:
                headers["authorization"] = headers["authorization"].replace(place_holder, secret)
        answer = self.gateway.handle(curl.method, curl.url, headers, curl.body)
        self.assertIn(answer.status, (200, 201), "%s answered %s: %s" % (curl.route, answer.status, answer.body))
        return json.loads(answer.body)

    def test_every_curl_example_works_in_the_order_of_the_document(self) -> None:
        examples = curls()
        # The document shows the sign-up and each step of the loop.
        self.assertEqual(
            [curl.route for curl in examples],
            [
                "POST /v1/owners",
                "POST /v1/residents",
                "GET /v1/catalogue",
                "GET /v1/observe",
                "POST /v1/act",
                "POST /v1/act",
                "GET /v1/wait",
                "PUT /v1/routines",
            ],
        )
        signup, create, catalogue, observe, act, plan, wait, routines = examples

        # Sign-up: the owner token of the first answer creates the resident.
        self.secrets[OWNER_TOKEN] = self.send(signup)["token"]
        self.assertEqual(create.headers["authorization"], "Bearer " + OWNER_TOKEN)
        created = self.send(create)
        self.secrets[RESIDENT_KEY] = created["key"]
        resident = self.world.residents[created["resident"]]
        self.assertEqual((resident.name, resident.model), ("Ada", "my-model"))

        # The catalogue needs no key. Every other example sends the resident key.
        self.assertNotIn("authorization", catalogue.headers)
        for curl in (observe, act, plan, wait, routines):
            self.assertEqual(curl.headers["authorization"], "Bearer " + RESIDENT_KEY, curl.route)
        verbs = {verb["verb"] for verb in self.send(catalogue)["verbs"]}

        observation = self.send(observe)
        self.assertEqual(observation["resident"], resident.id)

        # One action: the example gets the acknowledgement that its comment shows.
        acted = self.send(act)
        self.assertEqual(
            [(ack["verb"], ack["status"]["state"], ack["status"]["ticks"]) for ack in acted["acks"]],
            [("move_to", "started", 12)],
        )
        self.assertEqual(acted["acks"][0]["request_id"], json.loads(act.body)["id"])
        self.assertIn("observation", acted)
        # The same request again does not act again.
        self.assertEqual(self.send(act)["acks"], acted["acks"])

        # A plan: both steps go to the queue behind the walk.
        steps = json.loads(plan.body)["plan"]
        self.assertLessEqual({step["verb"] for step in steps}, verbs)
        planned = self.send(plan)
        self.assertEqual([ack["status"]["state"] for ack in planned["acks"]], ["queued", "queued"])

        # The long poll gets the cursor of the last observation, as the document says.
        # It returns with the report of the walk, before its timeout ends.
        self.assertRegex(wait.url, r"[?&]since=\d+(&|$)")
        cursor = "since=%d" % planned["observation"]["next_notice"]
        waited = self.send(wait._replace(url=re.sub(r"since=\d+", cursor, wait.url)))
        self.assertEqual([report["verb"] for report in waited["sections"]["reports"]], ["move_to"])
        self.assertEqual(self.world.tick, 12)

        # The routines are accepted as they are written.
        self.send(routines)
        self.assertEqual([routine["name"] for routine in resident.routines], ["eat", "sleep at night"])

    def test_the_examples_put_no_secret_in_a_url_or_a_body(self) -> None:
        for curl in curls():
            for place_holder in (OWNER_TOKEN, RESIDENT_KEY):
                self.assertNotIn(place_holder, curl.url)
                self.assertNotIn(place_holder, curl.body)


class JsonExamplesTest(unittest.TestCase):
    def setUp(self) -> None:
        self.examples = [json.loads(block) for block in blocks("json")]
        self.schema = json.loads(SCHEMA.read_text(encoding="utf-8"))["$defs"]

    def required(self, name: str) -> set:
        return set(self.schema[name]["required"])

    def observation(self) -> Dict[str, Any]:
        found = [example for example in self.examples if "sections" in example]
        self.assertEqual(len(found), 1)
        return found[0]

    def test_the_example_observation_has_the_fields_of_the_schema(self) -> None:
        observation = self.observation()
        self.assertEqual(set(observation), set(self.schema["Observation"]["properties"]))
        self.assertLessEqual(self.required("Clock"), set(observation["clock"]))
        self.assertEqual(len(observation["notices"]), 1)
        for notice in observation["notices"]:
            self.assertLessEqual(self.required("Notice"), set(notice))
            self.assertLessEqual(set(notice), set(self.schema["Notice"]["properties"]))
            self.assertEqual(set(notice["data"]), set(self.schema["Untrusted"]["properties"]))
            self.assertIs(notice["data"]["untrusted"], True)
        self.assertEqual(len(observation["sections"]["reports"]), 1)
        for report in observation["sections"]["reports"]:
            self.assertLessEqual(self.required("ActionReport"), set(report))
            self.assertLessEqual(set(report), set(self.schema["ActionReport"]["properties"]))
        # Notices and reports share one sequence. The report of the example came after its notice.
        entries = len(observation["notices"]) + len(observation["sections"]["reports"])
        self.assertEqual(observation["next_notice"], observation["notices"][0]["seq"] + entries)

    def test_the_example_sections_have_the_keys_that_the_server_writes(self) -> None:
        example = self.observation()["sections"]
        world = town()
        ada = world.spawn(world.sign_up()["owner"], "Ada")
        world.spawn(world.sign_up()["owner"], "Bo")
        real = world.observe(ada)["sections"]
        self.assertEqual(set(example["self"]), set(real["self"]))
        self.assertEqual(set(example["self"]["site"]), set(real["self"]["site"]))
        self.assertEqual(set(example["nearby"]), set(real["nearby"]))
        self.assertEqual(set(example["nearby"]["residents"][0]), set(real["nearby"]["residents"][0]))
        self.assertEqual(set(example["nearby"]["sites"][0]), set(real["nearby"]["sites"][0]))

    def test_the_untrusted_example_is_the_form_of_the_schema(self) -> None:
        found = [example for example in self.examples if "untrusted" in example]
        self.assertEqual(len(found), 1)
        self.assertEqual(set(found[0]), set(self.schema["Untrusted"]["properties"]))
        self.assertIs(found[0]["untrusted"], True)

    def test_the_document_is_short(self) -> None:
        # An agent reads the whole document before it joins. Keep it below about 2,500 tokens.
        words = len(JOIN.read_text(encoding="utf-8").split())
        self.assertLess(words, 1700, "docs/join.md has %d words" % words)


if __name__ == "__main__":
    unittest.main()
