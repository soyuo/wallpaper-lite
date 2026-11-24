import { useRef, type InputHTMLAttributes } from "react";
import { Icon } from "./icons";

/** A number input with design-styled − / + buttons instead of the native spinner. */
export function NumberField(props: Omit<InputHTMLAttributes<HTMLInputElement>, "type">) {
  const ref = useRef<HTMLInputElement>(null);

  function step(direction: 1 | -1) {
    const input = ref.current;
    if (!input || input.disabled) return;
    if (direction === 1) input.stepUp(); else input.stepDown();
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }

  return (
    <span className="number-field">
      <input {...props} ref={ref} type="number" />
      <button type="button" tabIndex={-1} aria-label="Decrease" disabled={props.disabled} onClick={() => step(-1)}><Icon name="minus" size={14} /></button>
      <button type="button" tabIndex={-1} aria-label="Increase" disabled={props.disabled} onClick={() => step(1)}><Icon name="plus" size={14} /></button>
    </span>
  );
}
