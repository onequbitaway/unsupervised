"""The generated types must follow the schema."""

import json
import sys
import types as module_types
import typing
import unittest
from unittest import mock

import _path

import gen_types
from unsupervised import ERROR_CODE_VALUES, SIM_ERROR_VALUES, types


class GeneratedTypesTest(unittest.TestCase):
    def test_types_py_is_what_the_generator_makes_from_the_schema(self) -> None:
        actual = gen_types.OUTPUT_PATH.read_text(encoding="utf-8")
        self.assertTrue(
            actual == gen_types.generate_from_disk(),
            "unsupervised/types.py is stale. Run `python3 scripts/gen_types.py` in sdk/python.",
        )

    def test_the_error_codes_are_the_codes_of_the_schema(self) -> None:
        schema = json.loads((_path.REPOSITORY_ROOT / "schema" / "protocol.schema.json").read_text(encoding="utf-8"))

        def codes(name: str) -> tuple:
            return tuple(member["const"] for member in schema["$defs"][name]["oneOf"])

        self.assertEqual(len(ERROR_CODE_VALUES), 13)
        self.assertEqual(ERROR_CODE_VALUES, codes("ErrorCode"))
        self.assertEqual(SIM_ERROR_VALUES, codes("SimError"))

    def test_every_type_hint_resolves(self) -> None:
        schema = json.loads(gen_types.SCHEMA_PATH.read_text(encoding="utf-8"))
        for name in schema["$defs"]:
            self.assertIn(name, types.__all__)
        resolved = set()
        for name in types.__all__:
            value = getattr(types, name)
            if isinstance(value, type) and hasattr(value, "__required_keys__"):
                # A name that the module does not define raises NameError here.
                typing.get_type_hints(value)
                resolved.add(name)
        self.assertTrue({"Observation", "ActionAck", "Notice", "Catalogue", "ConditionAll"} <= resolved)
        hints = typing.get_type_hints(types.Notice)
        self.assertEqual(hints["seq"], int)
        self.assertEqual(hints["from"], typing.Optional[int])
        self.assertEqual(types.Notice.__required_keys__, frozenset({"seq", "tick", "kind"}))
        self.assertEqual(types.Notice.__optional_keys__, frozenset({"from", "data"}))
        self.assertEqual(typing.get_type_hints(types.Untrusted)["untrusted"], typing.Literal[True])
        self.assertEqual(typing.get_type_hints(types.ActionStatusRejected)["state"], typing.Literal["rejected"])

    def test_the_generator_translates_each_schema_form(self) -> None:
        text = gen_types.generate(
            {
                "x-wire-types": ["Id", "Kind", "Thing"],
                "$defs": {
                    "Thing": {
                        "description": "A thing. See [`Kind`].",
                        "type": "object",
                        "required": ["id", "kind"],
                        "properties": {
                            "from": {"anyOf": [{"$ref": "#/$defs/Id"}, {"type": "null"}]},
                            "id": {"$ref": "#/$defs/Id"},
                            "kind": {"$ref": "#/$defs/Kind"},
                            "pairs": {
                                "type": "array",
                                "items": {"type": "array", "prefixItems": [{"type": "string"}, {"type": "integer"}]},
                            },
                            "tags": {"type": ["array", "null"], "items": {"type": "string"}},
                            "data": {"description": "Open data."},
                            "counts": {"type": "object", "additionalProperties": {"type": "integer"}},
                            "flag": {"type": "boolean", "const": True},
                        },
                    },
                    "Kind": {"oneOf": [{"const": "a", "type": "string"}, {"const": "b", "type": "string"}]},
                    "Id": {"type": "integer", "format": "uint64", "minimum": 0},
                    "Shape": {
                        "oneOf": [
                            {"type": "object", "required": ["type"], "properties": {"type": {"const": "dot", "type": "string"}}},
                            {
                                "type": "object",
                                "required": ["type", "of"],
                                "properties": {
                                    "type": {"const": "group", "type": "string"},
                                    "of": {"type": "array", "items": {"$ref": "#/$defs/Shape"}},
                                },
                            },
                        ]
                    },
                },
            }
        )
        # The generated text runs as a module with a name: a type hint finds
        # the other types through the module of its class.
        module = module_types.ModuleType("generated_example")
        namespace = module.__dict__
        exec(compile(text, "generated", "exec"), namespace)
        registered = mock.patch.dict(sys.modules, {module.__name__: module})
        registered.start()
        self.addCleanup(registered.stop)
        self.assertEqual(namespace["__all__"], ["Value", "Id", "Kind", "KIND_VALUES", "Thing", "ShapeDot", "ShapeGroup", "Shape"])
        self.assertIs(namespace["Id"], int)
        self.assertEqual(namespace["Kind"], typing.Literal["a", "b"])
        self.assertEqual(namespace["KIND_VALUES"], ("a", "b"))
        thing = namespace["Thing"]
        self.assertEqual(thing.__doc__, "A thing. See `Kind`.")
        self.assertEqual(thing.__required_keys__, frozenset({"id", "kind"}))
        hints = typing.get_type_hints(thing, namespace)
        self.assertEqual(hints["id"], int)
        self.assertEqual(hints["kind"], typing.Literal["a", "b"])
        self.assertEqual(hints["from"], typing.Optional[int])
        self.assertEqual(hints["pairs"], typing.List[typing.List[typing.Union[str, int]]])
        self.assertEqual(hints["tags"], typing.Optional[typing.List[str]])
        self.assertEqual(hints["data"], typing.Any)
        self.assertEqual(hints["counts"], typing.Dict[str, int])
        self.assertEqual(hints["flag"], typing.Literal[True])
        group = typing.get_type_hints(namespace["ShapeGroup"], namespace)
        self.assertEqual(group["type"], typing.Literal["group"])
        self.assertEqual(group["of"], typing.List[typing.Union[namespace["ShapeDot"], namespace["ShapeGroup"]]])

    def test_the_generator_refuses_a_schema_keyword_that_it_does_not_know(self) -> None:
        with self.assertRaisesRegex(gen_types.SchemaError, r'Thing\.a: the schema keyword "pattern" is not supported'):
            gen_types.generate({"$defs": {"Thing": {"type": "object", "properties": {"a": {"type": "string", "pattern": "^a"}}}}})
        with self.assertRaisesRegex(gen_types.SchemaError, r'Thing: the schema keyword "allOf" is not supported'):
            gen_types.generate({"$defs": {"Thing": {"allOf": []}}})
        with self.assertRaisesRegex(gen_types.SchemaError, r"the schema has no `\$defs`"):
            gen_types.generate({})


if __name__ == "__main__":
    unittest.main()
