import assert from "node:assert/strict";
import test from "node:test";
import { EditorHistory, resolveEditorShortcut } from "../src/editor-history.mjs";

test("records complete snapshots and restores them through undo and redo", () => {
  const history = new EditorHistory();
  const initial = { value: 0, nested: { x: 0 } };
  history.reset(initial);
  history.record({ value: 1, nested: { x: 5 } });

  assert.deepEqual(history.undo(), initial);
  assert.deepEqual(history.redo(), { value: 1, nested: { x: 5 } });
});

test("coalesces a continuous gesture into one undo step", () => {
  const history = new EditorHistory();
  history.reset({ value: 0 });
  history.beginGesture("brightness-slider");
  history.update({ value: 1 });
  history.update({ value: 2 });
  history.update({ value: 3 });
  history.endGesture("brightness-slider");

  assert.deepEqual(history.undo(), { value: 0 });
  assert.equal(history.canUndo, false);
  assert.deepEqual(history.redo(), { value: 3 });
});

test("new edits clear redo and a reset snapshot is undoable", () => {
  const history = new EditorHistory();
  history.reset({ value: 0 });
  history.record({ value: 1 });
  history.undo();
  history.record({ value: 2 });
  assert.equal(history.canRedo, false);

  history.record({ value: 0 });
  assert.deepEqual(history.undo(), { value: 2 });
});

test("retains no more than the configured 100 history steps", () => {
  const history = new EditorHistory(100);
  history.reset({ value: 0 });
  for (let value = 1; value <= 101; value += 1) history.record({ value });

  for (let step = 0; step < 100; step += 1) history.undo();
  assert.deepEqual(history.current, { value: 1 });
  assert.equal(history.canUndo, false);
});

test("focus-aware shortcuts leave text controls alone", () => {
  const shortcut = (key, editableTarget = false, ctrlKey = true) => resolveEditorShortcut({
    key,
    ctrlKey,
    metaKey: false,
    shiftKey: false,
    editableTarget,
  });

  assert.equal(shortcut("z"), "undo");
  assert.equal(shortcut("y"), "redo");
  assert.equal(shortcut("Delete", false, false), "reset");
  assert.equal(shortcut("z", true), null);
  assert.equal(shortcut("Delete", true, false), null);
});
