export type EditorShortcut = "undo" | "redo" | "reset";

export class EditorHistory<T> {
  constructor(limit?: number);
  readonly current: T | null;
  readonly canUndo: boolean;
  readonly canRedo: boolean;
  reset(initial: T): T;
  clear(): void;
  record(next: T): T;
  beginGesture(key: string): void;
  update(next: T): T;
  endGesture(key?: string): T | null;
  undo(): T | null;
  redo(): T | null;
}

export function resolveEditorShortcut(input: {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  editableTarget: boolean;
}): EditorShortcut | null;
