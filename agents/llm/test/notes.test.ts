// The notes file.

import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, type TestContext } from "node:test";

import { DEFAULT_NOTES_MAX, FileNotes } from "../src/index.js";

async function directory(t: TestContext): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "unsupervised-notes-"));
  t.after(() => rm(path, { recursive: true, force: true }));
  return path;
}

test("a file that does not exist has no notes, and the first note makes it", async (t) => {
  const path = join(await directory(t), "notes.md");
  const notes = new FileNotes(path);
  assert.deepEqual(await notes.read(), []);

  await notes.add("The shelter is at 7,7.", "year 1 day 2 8:05");
  assert.equal(await readFile(path, "utf8"), "[year 1 day 2 8:05] The shelter is at 7,7.\n");
  assert.deepEqual(await notes.read(), ["[year 1 day 2 8:05] The shelter is at 7,7."]);
});

test("a second process reads the notes of the first", async (t) => {
  const path = join(await directory(t), "notes.md");
  await new FileNotes(path).add("Bread costs 250.", "year 1 day 1 9:00");
  await new FileNotes(path).add("Work starts at 8.", "year 1 day 1 9:30");
  assert.deepEqual(await new FileNotes(path).read(), [
    "[year 1 day 1 9:00] Bread costs 250.",
    "[year 1 day 1 9:30] Work starts at 8.",
  ]);
});

test("the file keeps the newest notes only", async (t) => {
  const path = join(await directory(t), "notes.md");
  const notes = new FileNotes(path, 3);
  for (let index = 1; index <= 5; index += 1) await notes.add(`fact ${index}`, `day ${index}`);
  assert.deepEqual(await notes.read(), ["[day 3] fact 3", "[day 4] fact 4", "[day 5] fact 5"]);
  assert.equal(await readFile(path, "utf8"), "[day 3] fact 3\n[day 4] fact 4\n[day 5] fact 5\n");
  assert.equal(DEFAULT_NOTES_MAX, 20);
});

test("one note is one line, and a file that a person edited is read without its empty lines", async (t) => {
  const path = join(await directory(t), "notes.md");
  const notes = new FileNotes(path);
  await notes.add("  first line\nsecond\tline  ", "day 1");
  assert.deepEqual(await notes.read(), ["[day 1] first line second line"]);

  await writeFile(path, "\n  my own note  \n\n[day 1] kept\n", "utf8");
  assert.deepEqual(await notes.read(), ["my own note", "[day 1] kept"]);
});

test("an error of the file system that is not a missing file is thrown", async (t) => {
  // A directory in the place of the file cannot be read as a file.
  const path = await directory(t);
  await assert.rejects(new FileNotes(path).read(), (error: unknown) => {
    assert.equal((error as NodeJS.ErrnoException).code, "EISDIR");
    return true;
  });
});
