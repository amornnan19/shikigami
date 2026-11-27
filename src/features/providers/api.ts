import { invoke } from "@tauri-apps/api/core";

export type SignedIn = "yes" | "no" | "unknown";

export interface ApiKeyStatus {
  saved: boolean;
  lastFour: string | null;
  enabled: boolean;
  readFailed: boolean;
}

export interface ApiKeyError {
  code: string;
  message: string;
}

export interface ProviderStatus {
  id: string;
  name: string;
  installed: boolean;
  path: string | null;
  version: string | null;
  signedIn: SignedIn;
  installHint: string;
  signInHint: string;
  apiKey: ApiKeyStatus | null;
}

export function listProviders() {
  return invoke<ProviderStatus[]>("list_providers");
}

export function asApiKeyError(error: unknown): ApiKeyError {
  if (
    typeof error === "object" &&
    error !== null &&
    typeof (error as ApiKeyError).code === "string" &&
    typeof (error as ApiKeyError).message === "string"
  ) {
    return error as ApiKeyError;
  }
  return { code: "unknown", message: String(error) };
}

export function saveApiKey(provider: string, key: string) {
  return invoke<ApiKeyStatus>("save_api_key", { provider, key });
}

export function removeApiKey(provider: string) {
  return invoke<ApiKeyStatus>("remove_api_key", { provider });
}

export function setApiKeyEnabled(provider: string, enabled: boolean) {
  return invoke<ApiKeyStatus>("set_api_key_enabled", { provider, enabled });
}
