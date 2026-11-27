import { invoke } from "@tauri-apps/api/core";

export type SignedIn = "yes" | "no" | "unknown";

export type AuthMode = "subscription" | "api-key";

export const AUTH_MODE_LABELS: Record<AuthMode, string> = {
  subscription: "Subscription",
  "api-key": "API key",
};

export interface ApiKeyStatus {
  saved: boolean;
  lastFour: string | null;
  readFailed: boolean;
}

export interface ApiKeyError {
  code: string;
  message: string;
}

export interface ClaudeAuth {
  appMode: AuthMode;
  appModeExplicit: boolean;
  subscription: {
    installed: boolean;
    version: string | null;
    signedIn: SignedIn;
    signInHint: string;
    installHint: string;
  };
  apiKey: ApiKeyStatus;
}

export const CLAUDE_AUTH_KEY = ["claude-auth"];

export function getClaudeAuth() {
  return invoke<ClaudeAuth>("get_claude_auth");
}

export function setClaudeAuthMode(mode: AuthMode) {
  return invoke<void>("set_claude_auth_mode", { mode });
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
