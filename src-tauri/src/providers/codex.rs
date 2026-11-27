use super::probe::ProbeOutput;
use super::{ProviderSpec, SignedIn};

pub const SPEC: ProviderSpec = ProviderSpec {
    id: "codex",
    name: "Codex",
    executable: "codex",
    install_hint: "https://github.com/openai/codex",
    sign_in_hint: "codex login",
    sign_in_args: &["login", "status"],
    interpret_sign_in,
};

fn interpret_sign_in(output: &ProbeOutput) -> SignedIn {
    if output.success {
        SignedIn::Yes
    } else {
        SignedIn::No
    }
}
