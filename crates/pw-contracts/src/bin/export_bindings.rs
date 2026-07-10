use pw_contracts::dto::{AppStatusDto, ConversationStateDto};
use ts_rs::{Config, TS};

fn main() -> Result<(), ts_rs::ExportError> {
    let repository_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("pw-contracts must be inside the repository crates directory");
    let output_directory = repository_root.join("packages/contracts/src/generated");
    let config = Config::new().with_out_dir(output_directory);

    ConversationStateDto::export(&config)?;
    AppStatusDto::export(&config)
}
