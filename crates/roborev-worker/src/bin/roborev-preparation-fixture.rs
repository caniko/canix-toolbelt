//! Native qualification entrypoint; excluded from production package features.
fn main() -> anyhow::Result<()> {
    anyhow::ensure!(std::env::args().skip(1).collect::<Vec<_>>() == ["__canix-roborev-prepare"]);
    canix_toolbelt_roborev_worker::helper_main()
}
