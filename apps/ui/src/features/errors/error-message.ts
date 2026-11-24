import { HostAdapterError } from "../../host-adapter";
import { canRetry, explainError } from "../../error-guidance.mjs";

/**
 * Text for a failed action: the backend's message plus what to do next.
 * `fallback` is used when the failure is not an app error with a known code.
 */
export function describeError(error: unknown, fallback: string): string {
  if (error instanceof HostAdapterError) {
    return explainError(error.code, error.message);
  }
  return error instanceof Error ? error.message : fallback;
}

/** True when the failure came from the app and trying the same action again may help. */
export function isRetryable(error: unknown): boolean {
  return error instanceof HostAdapterError && canRetry(error.code);
}
