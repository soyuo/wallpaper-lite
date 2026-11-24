import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { ERROR_GUIDANCE, canRetry, explainError, guidanceFor } from "../src/error-guidance.mjs";

const adapterSource = readFileSync(new URL("../src/host-adapter.ts", import.meta.url), "utf8");
const errorSource = readFileSync(new URL("../../../crates/core/src/error.rs", import.meta.url), "utf8");

function codesOfTypeUnion() {
  const union = /export type AppErrorCode =([\s\S]*?);/.exec(adapterSource);
  assert.ok(union, "AppErrorCode union not found in host-adapter.ts");
  return [...union[1].matchAll(/"([a-z_]+)"/g)].map((match) => match[1]);
}

function codesOfTypeGuard() {
  const guard = /function isAppErrorCode[\s\S]*?\n\}/.exec(adapterSource);
  assert.ok(guard, "isAppErrorCode not found in host-adapter.ts");
  return [...guard[0].matchAll(/value === "([a-z_]+)"/g)].map((match) => match[1]);
}

function codesOfRustEnum() {
  // Only the arms of ErrorCode::as_str, so other string matches in the file are ignored.
  const start = errorSource.indexOf("pub const fn as_str(self)");
  assert.ok(start >= 0, "ErrorCode::as_str not found in error.rs");
  const end = errorSource.indexOf("\n    }\n}", start);
  assert.ok(end > start, "end of ErrorCode::as_str not found in error.rs");
  const codes = [...errorSource.slice(start, end).matchAll(/=> "([a-z_]+)",/g)].map((match) => match[1]);
  assert.ok(codes.length > 30, "too few error codes parsed from error.rs");
  return codes;
}

test("every error code the backend can send has guidance", () => {
  for (const code of codesOfRustEnum()) {
    assert.ok(guidanceFor(code), `no guidance for Rust error code ${code}`);
  }
});

test("the TypeScript error type, its guard and the Rust enum list the same codes", () => {
  const rust = [...codesOfRustEnum()].sort();
  assert.deepEqual([...codesOfTypeUnion()].sort(), rust, "AppErrorCode union differs from the Rust ErrorCode");
  assert.deepEqual([...codesOfTypeGuard()].sort(), rust, "isAppErrorCode differs from the Rust ErrorCode");
});

test("guidance has no entries for codes that do not exist", () => {
  const known = new Set(codesOfRustEnum());
  for (const code of Object.keys(ERROR_GUIDANCE)) {
    assert.ok(known.has(code), `guidance for unknown code ${code}`);
  }
});

test("guidance gives a next step and never exposes diagnostics", () => {
  for (const [code, guidance] of Object.entries(ERROR_GUIDANCE)) {
    assert.match(guidance.next, /[.!]$/, `${code}: next step must be a finished sentence`);
    assert.ok(guidance.next.length > 20, `${code}: next step is too short to help`);
    assert.doesNotMatch(guidance.next, /[A-Za-z]:\\|0x[0-9a-f]+|HRESULT|panicked|at .*\.rs/i, `${code}: leaks a diagnostic`);
    assert.equal(typeof guidance.retry, "boolean");
  }
});

test("explainError appends the next step and leaves unknown codes alone", () => {
  assert.equal(
    explainError("timeout", "The host did not answer."),
    "The host did not answer. The host may be busy. Wait a moment and try again.",
  );
  assert.equal(explainError("not_a_code", "Something happened."), "Something happened.");
  assert.equal(explainError("toString", "Prototype names are not codes."), "Prototype names are not codes.");
});

test("only errors where trying again can help are retryable", () => {
  assert.equal(canRetry("host_unavailable"), true);
  assert.equal(canRetry("host_launch_failed"), true);
  assert.equal(canRetry("invalid_editor_settings"), false);
  assert.equal(canRetry("unsupported_platform"), false);
  assert.equal(canRetry("not_a_code"), false);
});
