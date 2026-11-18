#!/usr/bin/env python3
"""Writes unsupervised/types.py from schema/protocol.schema.json.

    python3 scripts/gen_types.py           write the file
    python3 scripts/gen_types.py --check   fail when the file is stale

The schema comes from the Rust wire types (`make schema`). This script uses
the standard library only. It stops with an error when the schema uses a
keyword that it does not know, so a change in the schema format cannot pass
unseen.
"""

import json
import re
import sys
from pathlib import Path
from typing import Any, Dict, List, Tuple

PACKAGE_ROOT = Path(__file__).resolve().parent.parent
SCHEMA_PATH = PACKAGE_ROOT.parent.parent / "schema" / "protocol.schema.json"
OUTPUT_PATH = PACKAGE_ROOT / "unsupervised" / "types.py"

KNOWN_KEYWORDS = {
    "$ref",
    "additionalProperties",
    "anyOf",
    "const",
    "default",
    "description",
    "enum",
    "format",
    "items",
    "maxItems",
    "maximum",
    "minItems",
    "minimum",
    "oneOf",
    "prefixItems",
    "properties",
    "required",
    "title",
    "type",
}

IDENTIFIER = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
REF_PREFIX = "#/$defs/"
PRIMITIVES = {"null": "None", "boolean": "bool", "string": "str", "integer": "int", "number": "float"}

Schema = Dict[str, Any]


class SchemaError(Exception):
    """The schema has a form that this script cannot translate."""


def check_keywords(node: Schema, where: str) -> None:
    """Stops when a schema node has a keyword that this script does not know."""
    for key in node:
        if key not in KNOWN_KEYWORDS:
            raise SchemaError('%s: the schema keyword "%s" is not supported' % (where, key))


def clean_doc(text: str) -> str:
    """Turns Rust documentation links such as [`Name`] into `Name`."""
    return re.sub(r"\[(`[^`]+`)\]", r"\1", text)


def comment(node: Schema, indent: str) -> str:
    """Returns the description of a node as comment lines."""
    text = node.get("description")
    if not isinstance(text, str):
        return ""
    lines = clean_doc(text).split("\n")
    return "".join((indent + "# " + line).rstrip() + "\n" for line in lines)


def docstring(node: Schema) -> str:
    """Returns the description of a node as the docstring of a class body."""
    text = node.get("description")
    if not isinstance(text, str):
        return "    pass\n"
    lines = clean_doc(text).replace("\\", "\\\\").replace('"""', "'''").split("\n")
    if len(lines) == 1:
        return '    """%s"""\n' % lines[0]
    body = "".join(("    " + line).rstrip() + "\n" for line in lines[1:])
    return '    """%s\n%s    """\n' % (lines[0], body)


def wrapped(prefix: str, opener: str, members: List[str], closer: str) -> str:
    """Returns `prefix opener members closer` on one line, or one member on each line when that is long."""
    line = "%s%s%s%s" % (prefix, opener, ", ".join(members), closer)
    if len(line) <= 100:
        return line
    return "%s%s\n%s%s" % (prefix, opener, "".join("    %s,\n" % member for member in members), closer)


def literal(value: Any) -> str:
    """Returns the Python source of a JSON constant."""
    if isinstance(value, str):
        return json.dumps(value)
    return repr(value)


def union(members: List[str]) -> str:
    """Joins the members of a union and removes repeats."""
    unique = list(dict.fromkeys(members))
    if "None" in unique and len(unique) > 1:
        rest = [member for member in unique if member != "None"]
        inner = rest[0] if len(rest) == 1 else "Union[%s]" % ", ".join(rest)
        return "Optional[%s]" % inner
    return unique[0] if len(unique) == 1 else "Union[%s]" % ", ".join(unique)


def camel(name: str) -> str:
    """Returns `snake_case` as `CamelCase`."""
    return "".join(part.capitalize() for part in re.split(r"[^A-Za-z0-9]+", name) if part)


class Generator:
    """Collects the declarations of one output file."""

    def __init__(self) -> None:
        self.parts: List[str] = []

    def type_of(self, node: Any, where: str, name_hint: str) -> str:
        """Returns the type hint of a schema node, as Python source.

        A named type is written as a string, so that the order of the
        declarations does not matter. `name_hint` names the `TypedDict` that
        an object inside another type needs.
        """
        if node is True:
            return "Any"
        if not isinstance(node, dict):
            raise SchemaError("%s: the schema `%r` is not supported" % (where, node))
        check_keywords(node, where)
        if "$ref" in node:
            ref = node["$ref"]
            if not isinstance(ref, str) or not ref.startswith(REF_PREFIX):
                raise SchemaError('%s: the reference "%s" is not supported' % (where, ref))
            return json.dumps(ref[len(REF_PREFIX):])
        if "const" in node:
            return "Literal[%s]" % literal(node["const"])
        if isinstance(node.get("enum"), list):
            return "Literal[%s]" % ", ".join(literal(value) for value in node["enum"])
        alternatives = node.get("oneOf", node.get("anyOf"))
        if isinstance(alternatives, list):
            return union(
                [self.type_of(member, "%s[%d]" % (where, i), "%s%d" % (name_hint, i)) for i, member in enumerate(alternatives)]
            )
        kind = node.get("type")
        if isinstance(kind, list):
            return union([self.type_of(dict(node, type=one), where, name_hint) for one in kind])
        if kind is None:
            return "Any"
        if kind in PRIMITIVES:
            return PRIMITIVES[kind]
        if kind == "array":
            if isinstance(node.get("prefixItems"), list):
                items = [
                    self.type_of(item, "%s[%d]" % (where, i), "%s%d" % (name_hint, i))
                    for i, item in enumerate(node["prefixItems"])
                ]
                # JSON gives a list, so a fixed-length array is a list of its item types.
                return "List[%s]" % union(items)
            if "items" not in node:
                return "List[Any]"
            return "List[%s]" % self.type_of(node["items"], where + "[]", name_hint + "Item")
        if kind == "object":
            if "properties" in node:
                self.typed_dict(name_hint, node, where)
                return json.dumps(name_hint)
            extra = node.get("additionalProperties", True)
            if extra is True:
                return "Dict[str, Any]"
            if extra is False:
                return "Dict[str, Any]"
            return "Dict[str, %s]" % self.type_of(extra, where + ".*", name_hint + "Value")
        raise SchemaError('%s: the type "%s" is not supported' % (where, kind))

    def fields(self, name: str, node: Schema, where: str, names: List[str]) -> str:
        """Returns the body of a `TypedDict` call for some properties."""
        out = "{\n"
        for key in names:
            prop = node["properties"][key]
            out += comment(prop, "        ") if isinstance(prop, dict) else ""
            hint = self.type_of(prop, "%s.%s" % (where, key), name + camel(key))
            out += "        %s: %s,\n" % (json.dumps(key), hint)
        return out + "    }"

    def typed_dict(self, name: str, node: Schema, where: str) -> None:
        """Adds the `TypedDict` of an object with properties.

        Keys can be Python keywords (`from`), so the call form is used. A
        type with required and optional keys has one base for each.
        """
        if not IDENTIFIER.match(name):
            raise SchemaError('the type name "%s" is not an identifier' % name)
        properties = node.get("properties", {})
        required = list(node.get("required", []))
        for key in required:
            if key not in properties:
                raise SchemaError('%s: the required property "%s" has no schema' % (where, key))
        optional = [key for key in properties if key not in required]
        if required and optional:
            parts = (
                '_%sRequired = TypedDict(\n    "_%sRequired",\n    %s,\n)\n'
                % (name, name, self.fields(name, node, where, required))
                + '_%sOptional = TypedDict(\n    "_%sOptional",\n    %s,\n    total=False,\n)\n'
                % (name, name, self.fields(name, node, where, optional))
                + "\n\nclass %s(_%sRequired, _%sOptional):\n%s" % (name, name, name, docstring(node))
            )
        else:
            total = "" if required or not optional else "    total=False,\n"
            body = self.fields(name, node, where, required or optional)
            parts = '%s%s = TypedDict(\n    "%s",\n    %s,\n%s)\n' % (comment(node, ""), name, name, body, total)
        self.parts.append(parts)

    def declare(self, name: str, node: Schema) -> None:
        """Adds the declaration of one named wire type."""
        if not IDENTIFIER.match(name):
            raise SchemaError('the type name "%s" is not an identifier' % name)
        check_keywords(node, name)
        alternatives = node.get("oneOf", node.get("anyOf"))
        if isinstance(alternatives, list) and all(isinstance(m.get("const"), str) for m in alternatives):
            # An enum without data: a literal type and a tuple of its values.
            out = comment(node, "")
            for member in alternatives:
                check_keywords(member, name)
                doc = member.get("description", "")
                out += "#   %s: %s\n" % (member["const"], clean_doc(doc).replace("\n", " ")) if doc else ""
            values = [json.dumps(member["const"]) for member in alternatives]
            out += wrapped("%s = " % name, "Literal[", values, "]") + "\n"
            constant = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", name).upper() + "_VALUES"
            out += "\n# Every value of `%s`, in the order of the protocol document.\n" % name
            out += "%s: Tuple[str, ...] = (\n%s)\n" % (constant, "".join("    %s,\n" % value for value in values))
            self.parts.append(out)
            return
        if isinstance(alternatives, list):
            # An enum with data: one `TypedDict` for each variant and a union.
            members = []
            for i, member in enumerate(alternatives):
                check_keywords(member, "%s[%d]" % (name, i))
                members.append(self.type_of(member, "%s[%d]" % (name, i), name + self.variant(member, i)))
            self.parts.append("%s%s\n" % (comment(node, ""), wrapped("%s = " % name, "Union[", members, "]")))
            return
        if node.get("type") == "object" and "properties" in node:
            self.typed_dict(name, node, name)
            return
        plain = {key: value for key, value in node.items() if key != "description"}
        self.parts.append("%s%s = %s\n" % (comment(node, ""), name, self.type_of(plain, name, name)))

    @staticmethod
    def variant(member: Schema, index: int) -> str:
        """Returns the name part of one variant of an enum with data."""
        properties = member.get("properties", {})
        for prop in properties.values():
            if isinstance(prop, dict) and isinstance(prop.get("const"), str):
                return camel(prop["const"])
        required = member.get("required", [])
        return camel(required[0]) if required else str(index)


HEADER = '''# Generated by scripts/gen_types.py from schema/protocol.schema.json.
# Do not edit. Run `python3 scripts/gen_types.py` after the schema changes.
"""Wire types of Unsupervised as type hints.

The server sends and receives JSON. The client gives it as plain `dict`,
`list`, `str`, `int` and `bool` values. These types describe that JSON for
editors and type checkers. They do nothing at run time.
"""

from typing import Any, Dict, List, Literal, Optional, Tuple, TypedDict, Union

# A JSON value. Open payloads use it: arguments, results, notice data, sections.
Value = Any
'''


def generate(schema: Schema) -> str:
    """Returns the text of the generated file."""
    defs = schema.get("$defs")
    if not isinstance(defs, dict):
        raise SchemaError("the schema has no `$defs`")
    listed = [name for name in schema.get("x-wire-types", []) if name in defs]
    order = listed + sorted(name for name in defs if name not in listed)
    generator = Generator()
    for name in order:
        generator.declare(name, defs[name])
    names = ["Value"] + order
    footer = "__all__ = [\n%s]\n" % "".join("    %s,\n" % json.dumps(name) for name in exported(generator.parts, names))
    return "\n\n".join([HEADER.rstrip("\n") + "\n"] + [part.rstrip("\n") + "\n" for part in generator.parts] + [footer])


def exported(parts: List[str], names: List[str]) -> List[str]:
    """Returns every public name that the parts define, in order."""
    found = list(names[:1])
    for part in parts:
        for match in re.finditer(r"^(?:class )?([A-Z][A-Za-z0-9_]*)(?:\(|:| =)", part, re.MULTILINE):
            if match.group(1) not in found:
                found.append(match.group(1))
    return found


def generate_from_disk() -> str:
    """Reads the schema file and returns the text that the output file must have."""
    return generate(json.loads(SCHEMA_PATH.read_text(encoding="utf-8")))


def main(argv: List[str]) -> int:
    expected = generate_from_disk()
    if "--check" in argv:
        actual = OUTPUT_PATH.read_text(encoding="utf-8") if OUTPUT_PATH.exists() else ""
        if actual != expected:
            print("%s is stale. Run `python3 scripts/gen_types.py`." % OUTPUT_PATH, file=sys.stderr)
            return 1
        print("%s is current." % OUTPUT_PATH)
        return 0
    OUTPUT_PATH.write_text(expected, encoding="utf-8")
    print("wrote %s" % OUTPUT_PATH)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
