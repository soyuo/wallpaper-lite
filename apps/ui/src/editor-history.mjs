function clone(value) {
  return structuredClone(value);
}

function equal(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

export class EditorHistory {
  past = [];
  future = [];
  value = null;
  gesture = null;
  limit;

  constructor(limit = 100) {
    if (!Number.isInteger(limit) || limit < 1) {
      throw new RangeError("Editor history limit must be a positive integer.");
    }
    this.limit = limit;
  }

  get current() {
    return this.value === null ? null : clone(this.value);
  }

  get canUndo() {
    return this.past.length > 0 || this.gestureChanged();
  }

  get canRedo() {
    return this.future.length > 0 && !this.gestureChanged();
  }

  reset(initial) {
    this.past = [];
    this.future = [];
    this.gesture = null;
    this.value = clone(initial);
    return clone(this.value);
  }

  clear() {
    this.past = [];
    this.future = [];
    this.gesture = null;
    this.value = null;
  }

  record(next) {
    this.endGesture();
    if (this.value === null) {
      this.value = clone(next);
      return clone(this.value);
    }
    if (equal(this.value, next)) return clone(this.value);

    this.pushPast(this.value);
    this.future = [];
    this.value = clone(next);
    return clone(this.value);
  }

  beginGesture(key) {
    if (this.value === null) return;
    if (this.gesture?.key === key) return;
    this.endGesture();
    this.gesture = { key, before: clone(this.value) };
  }

  update(next) {
    if (this.value === null) return this.reset(next);
    if (this.gesture) {
      this.value = clone(next);
      return clone(this.value);
    }
    return this.record(next);
  }

  endGesture(key) {
    if (!this.gesture || (key !== undefined && this.gesture.key !== key)) {
      return this.current;
    }
    const { before } = this.gesture;
    this.gesture = null;
    if (this.value !== null && !equal(before, this.value)) {
      this.pushPast(before);
      this.future = [];
    }
    return this.current;
  }

  undo() {
    this.endGesture();
    const previous = this.past.pop();
    if (previous === undefined || this.value === null) return this.current;
    this.future.push(clone(this.value));
    this.value = clone(previous);
    return clone(this.value);
  }

  redo() {
    this.endGesture();
    const next = this.future.pop();
    if (next === undefined || this.value === null) return this.current;
    this.pushPast(this.value);
    this.value = clone(next);
    return clone(this.value);
  }

  gestureChanged() {
    return this.gesture !== null
      && this.value !== null
      && !equal(this.gesture.before, this.value);
  }

  pushPast(snapshot) {
    this.past.push(clone(snapshot));
    if (this.past.length > this.limit) this.past.shift();
  }
}

export function resolveEditorShortcut(input) {
  if (input.editableTarget) return null;
  const modifier = input.ctrlKey || input.metaKey;
  const key = input.key.toLowerCase();
  if (modifier && key === "z") return input.shiftKey ? "redo" : "undo";
  if (modifier && key === "y") return "redo";
  if (!modifier && !input.shiftKey && input.key === "Delete") return "reset";
  return null;
}
