// The options of the scripts on the command line.

/** Reads `--name value` pairs and `--flag` from the command line. */
export function readArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i++) {
    const word = argv[i];
    if (!word.startsWith('--')) throw new Error(`Not an option: ${word}`);
    const name = word.slice(2);
    const next = argv[i + 1];
    if (next === undefined || next.startsWith('--')) args[name] = true;
    else {
      args[name] = next;
      i += 1;
    }
  }
  return args;
}

/** Reads a whole number option, or gives the default. */
export function number(value, fallback) {
  if (value === undefined) return fallback;
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) throw new Error(`Not a number: ${value}`);
  return parsed;
}
