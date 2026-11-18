// The memory of the agent: a small text file with one note on each line.

import { readFile, writeFile } from "node:fs/promises";

/** Where the agent keeps its notes. */
export interface NotesStore {
  /** Returns the notes, oldest first. */
  read(): Promise<string[]>;
  /** Adds one note with a time stamp and drops the oldest notes above the limit. */
  add(text: string, stamp: string): Promise<void>;
}

/** Greatest number of notes that the file keeps. */
export const DEFAULT_NOTES_MAX = 20;

/** Notes in a text file. The file is small: it holds the newest notes only. */
export class FileNotes implements NotesStore {
  readonly #path: string;
  readonly #max: number;

  /** Makes a store for the file at `path`. The file is made with the first note. */
  constructor(path: string, max = DEFAULT_NOTES_MAX) {
    this.#path = path;
    this.#max = max;
  }

  /** Returns the notes, oldest first. A file that does not exist has no notes. */
  async read(): Promise<string[]> {
    let text: string;
    try {
      text = await readFile(this.#path, "utf8");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
      throw error;
    }
    return text
      .split("\n")
      .map((entry) => entry.trim())
      .filter((entry) => entry !== "")
      .slice(-this.#max);
  }

  /** Adds `[stamp] text` as the newest note. */
  async add(text: string, stamp: string): Promise<void> {
    const notes = [...(await this.read()), `[${stamp}] ${text.replace(/\s+/g, " ").trim()}`].slice(-this.#max);
    await writeFile(this.#path, `${notes.join("\n")}\n`, "utf8");
  }
}
