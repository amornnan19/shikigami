import { invoke } from "@tauri-apps/api/core";

export type SignedIn = "yes" | "no" | "unknown";

export interface ProviderStatus {
  id: string;
  name: string;
  installed: boolean;
  path: string | null;
  version: string | null;
  signedIn: SignedIn;
  installHint: string;
  signInHint: string;
}

export function listProviders() {
  return invoke<ProviderStatus[]>("list_providers");
}
