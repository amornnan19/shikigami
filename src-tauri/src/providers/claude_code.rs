use super::probe::ProbeOutput;
use super::{ProviderSpec, SignedIn};

pub const SPEC: ProviderSpec = ProviderSpec {
    id: "claude-code",
    name: "Claude Code",
    executable: "claude",
    install_hint: "https://docs.anthropic.com/en/docs/claude-code/setup",
    sign_in_hint: "claude auth login",
    sign_in_args: &["auth", "status"],
    interpret_sign_in,
};

fn interpret_sign_in(output: &ProbeOutput) -> SignedIn {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.stdout.trim()) else {
        return SignedIn::Unknown;
    };
    match value.get("loggedIn").and_then(serde_json::Value::as_bool) {
        Some(true) => SignedIn::Yes,
        Some(false) => SignedIn::No,
        None => SignedIn::Unknown,
    }
}
