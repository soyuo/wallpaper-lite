/** A failure message that screen readers announce, with an optional retry. */
export function ErrorNotice({ message, onRetry, retrying = false }: {
  message: string | null;
  onRetry?: () => void;
  retrying?: boolean;
}) {
  if (message === null) return null;
  return (
    <div className="error-notice" role="alert">
      <p>{message}</p>
      {onRetry && (
        <button disabled={retrying} onClick={onRetry}>{retrying ? "Trying again..." : "Try again"}</button>
      )}
    </div>
  );
}
