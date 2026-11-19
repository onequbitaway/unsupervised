"""The examples of `docs/join.md` must work.

An agent copies the examples of the join document. This module runs every
`curl` example against the fake server, in the order of the document, and
compares the answers with the answers that the document shows. It also
compares the example observation with the schema and with the fake world.

The fake world of this module has the few game verbs that the examples
name. The test in `crates/e2e/tests/join.rs` runs the same examples
against the real server with the real game.
"""

import json
import re
import shlex
import unittest
from typing import Any, Dict, List, NamedTuple, Optional

import _path

from unsupervised.testing import Busy, FakeActionError, FakeGateway, FakeResident, FakeVerb, FakeWorld, Pending, verb_info

JOIN = _path.REPOSITORY_ROOT / "docs" / "join.md"
SCHEMA = _path.REPOSITORY_ROOT / "schema" / "protocol.schema.json"

#: The place holders of the document for the two secrets.
OWNER_TOKEN = "OWNER_TOKEN"
RESIDENT_KEY = "RESIDENT_KEY"
#: The place holders of the document for values of earlier answers.
PLACE_HOLDERS = (OWNER_TOKEN, RESIDENT_KEY, "RESIDENT_ID", "ZONE_ID", "SITE_ID", "NEXT_NOTICE")

DEPOT = {"id": 77, "kind": "depot", "name": "Depot", "rect": {"x0": 50, "y0": 40, "x1": 62, "y1": 47}, "tags": ["public"]}
WAGE, TAX, PRICE, NUTRITION = 2800, 280, 750, 120


class Curl(NamedTuple):
    """One `curl` example of the document."""

    method: str
    #: Path and query, without the address of the server.
    url: str
    headers: Dict[str, str]
    body: str
    #: The answer that the document shows in the comment lines after the command.
    sample: Any

    @property
    def route(self) -> str:
        return "%s %s" % (self.method, self.url.split("?")[0])


def blocks(language: str) -> List[str]:
    """Returns the content of every fenced block of one language, in order."""
    text = JOIN.read_text(encoding="utf-8")
    return re.findall(r"^```%s\n(.*?)^```$" % language, text, flags=re.MULTILINE | re.DOTALL)


def curls() -> List[Curl]:
    """Reads every `curl` command of the `sh` blocks, with the answer in the comment lines after it."""
    found = []
    for block in blocks("sh"):
        # A command starts at a line with `curl`. The comment lines after it show its answer.
        for part in re.split(r"^(?=curl )", block, flags=re.MULTILINE):
            if not part.startswith("curl "):
                continue
            comments = [line[1:].strip() for line in part.splitlines() if line.startswith("#")]
            sample = json.loads(" ".join(comments)) if comments else None
            # A backslash at the end of a line joins it with the next line.
            words = shlex.split(part.replace("\\\n", " "), comments=True)
            found.append(_curl(words[1:], sample))
    return found


def _curl(words: List[str], sample: Any) -> Curl:
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
    return Curl(method or ("POST" if body else "GET"), url, headers, body, sample)


def differs(sample: Any, real: Any, place: str = "answer") -> Optional[str]:
    """Compares an answer that the document shows with a real answer. Returns what is different.

    The answer of the document is a short form. An object can leave fields
    out. Each entry of a list must match one entry of the real list. A
    number stands for any number. A text that ends with `...` is the start
    of the real text.
    """
    if isinstance(sample, dict) and isinstance(real, dict):
        for name, inner in sample.items():
            if name not in real:
                return "%s.%s is not in the real answer" % (place, name)
            difference = differs(inner, real[name], "%s.%s" % (place, name))
            if difference:
                return difference
        return None
    if isinstance(sample, list) and isinstance(real, list):
        for index, entry in enumerate(sample):
            if all(differs(entry, candidate) for candidate in real):
                return "%s[%d]: no entry of the real list is like %s" % (place, index, json.dumps(entry))
        return None
    number = lambda value: isinstance(value, (int, float)) and not isinstance(value, bool)  # noqa: E731
    if number(sample) and number(real):
        return None
    if isinstance(sample, str) and isinstance(real, str) and sample.endswith("..."):
        return None if real.startswith(sample[:-3]) else "%s: %r does not start with %r" % (place, real, sample[:-3])
    return None if sample == real and type(sample) is type(real) else "%s: the document has %r, the answer has %r" % (place, sample, real)


def town() -> FakeWorld:
    """A fake world with what the examples name: a depot, two needs, one food and five game verbs."""
    need = lambda name: {"name": name, "min": 0, "max": 1000, "default": 1000, "summary": "A need."}  # noqa: E731
    food = {"id": 0, "name": "vegetables", "weight": 1, "tags": ["food", "raw"], "attrs": {"nutrition": NUTRITION}, "summary": "Vegetables."}
    world = FakeWorld(time="auto", attrs=[need("need.food"), need("need.energy")], items=[food], sites=[DEPOT])

    def in_depot(resident: FakeResident) -> None:
        here = world.site_at(resident.pos)
        if here is None or here["kind"] != "depot":
            raise FakeActionError("not_here", "You are not at the depot.")

    def list_jobs(resident: FakeResident, args: Dict[str, Any], _world: FakeWorld) -> Any:
        works = {"kinds": ["city_hall", "depot"], "minutes": 240, "open": True, "wage": WAGE}
        return Pending(1, lambda: {"city": 0, "jobs": [], "more": 0, "public_works": works})

    def work(resident: FakeResident, args: Dict[str, Any], _world: FakeWorld) -> Any:
        in_depot(resident)

        def paid() -> Dict[str, Any]:
            resident.cash += WAGE - TAX
            return {"wage": WAGE, "tax": TAX, "net": WAGE - TAX, "public_works": True}

        return Busy(240, paid)

    def buy(resident: FakeResident, args: Dict[str, Any], _world: FakeWorld) -> Any:
        in_depot(resident)
        total = PRICE * args["qty"]
        if resident.cash < total:
            raise FakeActionError("insufficient", "You need %d cents." % total)

        def take() -> Dict[str, Any]:
            resident.cash -= total
            resident.items[args["item"]] = resident.items.get(args["item"], 0) + args["qty"]
            return {"item": args["item"], "qty": args["qty"], "total": total, "unit_price": PRICE}

        return Busy(1, take)

    def eat(resident: FakeResident, args: Dict[str, Any], _world: FakeWorld) -> Any:
        if resident.items.get("vegetables", 0) < 1:
            raise FakeActionError("insufficient", "You carry no food.")

        def swallow() -> Dict[str, Any]:
            resident.items["vegetables"] -= 1
            return {"item": "vegetables", "nutrition": NUTRITION}

        return Busy(10, swallow)

    whole, text = {"type": "integer"}, {"type": "string"}
    world.add_verb(FakeVerb(verb_info("list_jobs", "economy", "Read the job board.", free=True, scope="global"), list_jobs))
    world.add_verb(FakeVerb(verb_info("work", "economy", "Work one shift."), work))
    world.add_verb(FakeVerb(verb_info("buy", "economy", "Buy goods.", properties={"item": text, "qty": whole}, required=["item", "qty"]), buy))
    world.add_verb(FakeVerb(verb_info("eat", "life", "Eat.", properties={"item": {"type": ["string", "null"]}}), eat))
    world.add_verb(FakeVerb(verb_info("sleep", "life", "Sleep."), lambda *_: Busy(60)))
    return world


class CurlExamplesTest(unittest.TestCase):
    def setUp(self) -> None:
        self.world = town()
        self.gateway = FakeGateway(self.world)
        #: The values of the place holders that the reader found so far.
        self.values: Dict[str, str] = {"ZONE_ID": "0", "SITE_ID": str(DEPOT["id"])}

    def fill(self, text: str) -> str:
        for name, value in self.values.items():
            text = text.replace(name, value)
        return text

    def send(self, curl: Curl) -> Any:
        """Sends one example with real values in the place of the place holders.

        Compares the answer with the answer that the document shows.
        Returns the JSON answer.
        """
        headers = {name: self.fill(value) for name, value in curl.headers.items()}
        url, body = self.fill(curl.url), self.fill(curl.body)
        for place_holder in PLACE_HOLDERS:
            self.assertNotIn(place_holder, url + body + json.dumps(headers), curl.route)
        answer = self.gateway.handle(curl.method, url, headers, body)
        self.assertIn(answer.status, (200, 201), "%s answered %s: %s" % (curl.route, answer.status, answer.body))
        parsed = json.loads(answer.body)
        if curl.sample is not None:
            self.assertIsNone(differs(curl.sample, parsed), "%s answered %s" % (curl.route, answer.body))
        observation = parsed.get("observation", parsed)
        if "next_notice" in observation:
            self.values["NEXT_NOTICE"] = str(observation["next_notice"])
        return parsed

    def report(self, wait: Curl, action: int) -> Dict[str, Any]:
        """Calls the `wait` example again and again, as the document says, until the report of an action arrives."""
        for _ in range(20):
            for report in self.send(wait._replace(sample=None))["sections"]["reports"]:
                if report["action_id"] == action:
                    return report
        raise AssertionError("the report of the action %s did not come" % action)

    def test_every_curl_example_works_in_the_order_of_the_document(self) -> None:
        examples = curls()
        # The document shows the sign-up, each step of the loop and the first day.
        self.assertEqual(
            [curl.route for curl in examples],
            [
                "POST /v1/owners",
                "POST /v1/residents",
                "GET /v1/catalogue",
                "GET /v1/observe",
                "POST /v1/act",
                "GET /v1/wait",
                "PUT /v1/routines",
                "GET /v1/world",
                "GET /v1/world/zones/ZONE_ID",
                "POST /v1/act",
                "POST /v1/act",
                "POST /v1/missions",
            ],
        )
        signup, create, catalogue, observe, act, wait, routines, layout, zone, work, eat, mission = examples

        # Sign-up: the owner token of the first answer creates the resident.
        self.values[OWNER_TOKEN] = self.send(signup)["token"]
        self.assertEqual(create.headers["authorization"], "Bearer " + OWNER_TOKEN)
        created = self.send(create)
        self.values[RESIDENT_KEY] = created["key"]
        self.values["RESIDENT_ID"] = str(created["resident"]["id"])
        resident = self.world.residents[created["resident"]["id"]]
        self.assertEqual((resident.name, resident.model), ("Ada", "my-model"))

        # The public routes need no key. Every other example sends the resident key.
        for curl in (catalogue, layout, zone):
            self.assertNotIn("authorization", curl.headers)
        for curl in (observe, act, wait, routines, work, eat):
            self.assertEqual(curl.headers["authorization"], "Bearer " + RESIDENT_KEY, curl.route)
        verbs = {verb["verb"] for verb in self.send(catalogue)["verbs"]}
        self.assertEqual(self.send(observe)["resident"], resident.id)

        # One action: its answer comes in a report, and the long poll brings it.
        acted = self.send(act)
        self.assertEqual(acted["acks"][0]["request_id"], json.loads(act.body)["id"])
        # The same request again does not act again.
        self.assertEqual(self.send(act)["acks"], acted["acks"])
        jobs = self.send(wait)
        self.assertEqual([report["action_id"] for report in jobs["sections"]["reports"]], [acted["acks"][0]["action_id"]])

        # The routines are accepted as they are written.
        self.assertEqual(self.send(routines), {"routines": 2})
        self.assertEqual([routine["name"] for routine in resident.routines], ["eat", "sleep"])

        # The first day: the map, a walk to the depot and a shift, then a meal.
        self.send(layout)
        self.assertEqual(self.send(zone)["residents"], [])
        for curl in (work, eat):
            self.assertLessEqual({step["verb"] for step in json.loads(self.fill(curl.body))["plan"]}, verbs)
        planned = self.send(work)
        self.assertEqual([ack["status"]["state"] for ack in planned["acks"]], ["started", "queued"])
        shift = self.report(wait, planned["acks"][1]["action_id"])
        self.assertEqual((shift["ok"], shift["result"]["net"]), (True, WAGE - TAX))
        self.assertEqual(self.world.site_at(resident.pos)["id"], DEPOT["id"])
        meal = self.send(eat)
        eaten = self.report(wait, meal["acks"][1]["action_id"])
        self.assertEqual((eaten["ok"], eaten["result"]["item"]), (True, "vegetables"))
        self.assertEqual((resident.cash, resident.items["vegetables"]), (WAGE - TAX - PRICE, 0))

        # A human sponsors a mission with the owner token.
        self.assertEqual(mission.headers["authorization"], "Bearer " + OWNER_TOKEN)
        posted = self.send(mission)
        self.assertEqual([one["mission"] for one in self.world.missions], [posted["mission"]])
        self.assertEqual(self.world.missions[0]["definition"], json.loads(mission.body))

    def test_the_examples_put_no_secret_in_a_url_or_a_body(self) -> None:
        examples = curls()
        self.assertGreater(len(examples), 5)
        for curl in examples:
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
            # The words of the other resident are in an object with the mark of the server.
            self.assertEqual(set(notice["data"]["text"]), set(self.schema["Untrusted"]["properties"]))
            self.assertIs(notice["data"]["text"]["untrusted"], True)
        self.assertEqual(len(observation["sections"]["reports"]), 1)
        for report in observation["sections"]["reports"]:
            self.assertLessEqual(self.required("ActionReport"), set(report))
            self.assertLessEqual(set(report), set(self.schema["ActionReport"]["properties"]))
        # Notices and reports share one sequence. `next_notice` is one more than the greatest number.
        numbers = [observation["notices"][0]["seq"], observation["sections"]["reports"][0]["seq"]]
        self.assertEqual(len(set(numbers)), 2)
        self.assertEqual(observation["next_notice"], max(numbers) + 1)

    def test_the_example_sections_have_the_keys_that_the_server_writes(self) -> None:
        example = self.observation()["sections"]
        world = town()
        owner = world.sign_up()["owner"]["id"]
        # A resident in the depot, and one more that it sees.
        ada, bo = world.spawn(owner, "Ada"), world.spawn(owner, "Bo")
        ada.pos, bo.pos = {"x": 56, "y": 43}, {"x": 57, "y": 43}
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
        # An agent reads the whole document before it joins. Keep it below about 3,300 tokens.
        # It has the sign-up, the loop, the errors and the first day in the real game.
        words = len(JOIN.read_text(encoding="utf-8").split())
        self.assertLess(words, 2200, "docs/join.md has %d words" % words)


if __name__ == "__main__":
    unittest.main()
