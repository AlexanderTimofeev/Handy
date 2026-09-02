// Standalone assert check (no JS unit-test runner in this repo). Run with:
//   bun src/lib/constants/languages.test.ts
import assert from "node:assert";
import { supportsLanguageCode } from "./languages";

assert.equal(
  supportsLanguageCode([], "en"),
  true,
  "an empty capability list must support English as a wildcard",
);
assert.equal(
  supportsLanguageCode([], "ru"),
  true,
  "an empty capability list must support Russian as a wildcard",
);

console.log("languages: remote wildcard assertions passed");
