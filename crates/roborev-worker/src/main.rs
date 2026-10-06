//! Trusted process boundary for Toolbelt's Linux Roborev worker.
fn main() -> anyhow::Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    match arguments.as_slice() {
        [argument] if argument == "__canix-roborev-prepare" => {
            canix_toolbelt_roborev_worker::helper_main()
        }
        [argument] if argument == "__canix-roborev-worker" => {
            canix_toolbelt_roborev_worker::offline::helper_main()
        }
        _ => anyhow::bail!("expected one private Roborev helper argument"),
    }
}
