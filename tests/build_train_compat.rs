#![cfg(all(unix, feature = "build-train"))]

use canix_toolbelt::build_train::{Connection, Service, policy_identity};

#[test]
fn native_contracts_share_fleetix_types_and_preserve_historical_policy() {
    let service: Service =
        serde_json::from_str(include_str!("fixtures/build-train-service.json")).unwrap();
    let native: fleetix::build_train::native::Service = service.clone();
    assert_eq!(
        policy_identity(&service).unwrap(),
        service.coordinator.policy
    );
    assert_eq!(
        fleetix::build_train::native::policy_identity(&native).unwrap(),
        service.coordinator.policy
    );
    let policy = service.coordinator.policy.clone();
    let connection = Connection {
        builder: service.builder,
        socket: service.coordinator.socket,
        policy: service.coordinator.policy,
        preparation_dir: "/var/lib/fleetix-train/preparation".into(),
        gc_roots: service.native.gc_roots,
    };
    let native: fleetix::build_train::native::Connection = connection;
    assert_eq!(native.client().policy, policy);
}

#[test]
#[cfg(feature = "cli")]
fn historical_cli_paths_accept_the_shared_fleetix_command_type() {
    let command = canix_toolbelt::build_train::cli::TrainCommand::Status {
        connection: "/etc/fleetix-train/connection.json".into(),
        attempt: Some("persisted-attempt".into()),
    };
    let _: fleetix::build_train::cli::TrainCommand = command;
    let _: fn(fleetix::build_train::cli::TrainCommand) -> Result<std::process::ExitCode, String> =
        canix_toolbelt::build_train::cli::run;
}
