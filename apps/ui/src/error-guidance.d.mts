export interface ErrorGuidance {
  /** What the user can do next. */
  next: string;
  /** Whether trying the same action again is likely to help. */
  retry: boolean;
}

export const ERROR_GUIDANCE: Readonly<Record<string, ErrorGuidance>>;
export function guidanceFor(code: string): ErrorGuidance | null;
export function explainError(code: string, message: string): string;
export function canRetry(code: string): boolean;
