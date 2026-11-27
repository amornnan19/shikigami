use super::probe::ProbeOutput;
use super::{ProviderSpec, SignedIn};

pub const SPEC: ProviderSpec = ProviderSpec {
    id: "codex",
    name: "Codex",
    executable: "codex",
    install_hint: "https://github.com/openai/codex",
    sign_in_hint: "codex login",
    sign_in_args: &["login", "status"],
    version_prefix: "codex-cli ",
    sign_in_floor: [0, 20, 0],
    interpret_sign_in,
};

fn interpret_sign_in(output: &ProbeOutput) -> SignedIn {
    match output.code {
        Some(0) => SignedIn::Yes,
        Some(1) => SignedIn::No,
        _ => SignedIn::Unknown,
    }
}
