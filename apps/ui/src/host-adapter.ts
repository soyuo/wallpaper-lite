import { invoke } from "@tauri-apps/api/core";

export interface HostAdapter {
  ping(): Promise<void>;
}

export const hostAdapter: HostAdapter = {
  ping: () => invoke<void>("host_ping"),
};
