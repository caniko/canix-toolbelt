//! Native offline-worker helper; excluded from production package features.
fn main() -> anyhow::Result<()> {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "launch")
    {
        use anyhow::{Context, ensure};
        use canix_toolbelt_roborev_worker::{
            execution::ExecutionFence,
            offline::{OfflineSpec, run_offline},
        };
        let args: Vec<_> = std::env::args_os().skip(2).collect();
        ensure!(
            args.len() == 3,
            "fixture requires original fence, offline spec and result path"
        );
        let fence: ExecutionFence =
            serde_json::from_slice(&std::fs::read(&args[0])?).context("fixture fence")?;
        let spec: OfflineSpec =
            serde_json::from_slice(&std::fs::read(&args[1])?).context("fixture spec")?;
        run_offline(fence.reserve()?, &spec, std::path::Path::new(&args[2]))?;
        return Ok(());
    }
    canix_toolbelt_roborev_worker::offline::helper_main()
}
