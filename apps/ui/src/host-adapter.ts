import { invoke } from "@tauri-apps/api/core";

export type AppErrorCode =
  | "host_unavailable"
  | "timeout"
  | "disconnected"
  | "malformed_message"
  | "malformed_request"
  | "malformed_response"
  | "message_too_large"
  | "protocol_version_mismatch"
  | "unsupported_platform"
  | "transport_error"
  | "internal";

interface AppErrorPayload {
  code: AppErrorCode;
  message: string;
}

export class HostAdapterError extends Error {
  constructor(
    readonly code: AppErrorCode,
    message: string,
  ) {
    super(message);
    this.name = "HostAdapterError";
  }
}

export interface HostAdapter {
  ping(): Promise<void>;
}

function isAppErrorPayload(value: unknown): value is AppErrorPayload {
  if (typeof value !== "object" || value === null) return false;

  const payload = value as Record<string, unknown>;
  return (
    isAppErrorCode(payload.code) &&
    typeof payload.message === "string"
  );
}

function isAppErrorCode(value: unknown): value is AppErrorCode {
  return (
    value === "host_unavailable" ||
    value === "timeout" ||
    value === "disconnected" ||
    value === "malformed_message" ||
    value === "malformed_request" ||
    value === "malformed_response" ||
    value === "message_too_large" ||
    value === "protocol_version_mismatch" ||
    value === "unsupported_platform" ||
    value === "transport_error" ||
    value === "internal"
  );
}

export const hostAdapter: HostAdapter = {
  async ping() {
    try {
      await invoke<void>("host_ping");
    } catch (error: unknown) {
      if (isAppErrorPayload(error)) {
        throw new HostAdapterError(
          error.code,
          error.message,
        );
      }

      throw new HostAdapterError(
        "internal",
        "Could not check the wallpaper host.",
      );
    }
  },
};
