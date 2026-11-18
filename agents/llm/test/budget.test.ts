// The budget of decisions for each hour.

import assert from "node:assert/strict";
import { test } from "node:test";

import { DecisionBudget } from "../src/index.js";

const MINUTE = 60_000;

test("the budget gives `perHour` decisions in any 60 minutes", () => {
  let now = 0;
  const budget = new DecisionBudget(3, () => now);
  assert.equal(budget.left, 3);
  assert.equal(budget.secondsUntilFree(), 0);

  assert.equal(budget.take(), true);
  now = 10 * MINUTE;
  assert.equal(budget.take(), true);
  now = 20 * MINUTE;
  assert.equal(budget.take(), true);
  assert.equal(budget.left, 0);
  assert.equal(budget.take(), false);
  // The first decision is one hour old in 40 minutes.
  assert.equal(budget.secondsUntilFree(), 2400);

  now = 60 * MINUTE - 1;
  assert.equal(budget.take(), false);
  assert.equal(budget.secondsUntilFree(), 1);

  now = 60 * MINUTE;
  assert.equal(budget.left, 1);
  assert.equal(budget.take(), true);
  assert.equal(budget.take(), false);
  // Now the decision of minute 10 is the oldest.
  assert.equal(budget.secondsUntilFree(), 600);

  now = 200 * MINUTE;
  assert.equal(budget.left, 3);
});

test("a budget of zero never gives a decision", () => {
  const budget = new DecisionBudget(0, () => 5 * MINUTE);
  assert.equal(budget.left, 0);
  assert.equal(budget.take(), false);
  assert.equal(budget.secondsUntilFree(), 3600);
});
