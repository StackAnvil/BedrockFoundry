#![cfg(unix)]

use bedrock_foundry::{
    server::{self, ServerConfig},
    storage::Store,
};
use std::{
    io::{BufRead, BufReader},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
};

fn foundry(data: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bedrock-foundry"));
    command.arg("--data-dir").arg(data);
    command
}

/// Creates an installed server whose BDS is a script that reports readiness and exits on `stop`.
#[expect(
    clippy::disallowed_methods,
    reason = "This synchronous fixture writes a fake BDS executable"
)]
fn fake_server(data: &Path) -> ServerConfig {
    let store = Store::open(Some(data.to_path_buf())).unwrap();
    let mut config = ServerConfig::new("Controller fixture".into(), 19140).unwrap();
    config.version = Some("1.26.60.6".into());
    server::save(&store.root, &config).unwrap();
    let home = server::home(&store.root, config.id);
    std::fs::write(
        home.join("bedrock_server"),
        b"#!/bin/sh\nprintf '[INFO] Server started.\\n'\nwhile read -r command; do\n  if [ \"$command\" = stop ]; then printf 'graceful exit' > stopped; exit 0; fi\ndone\n",
    )
    .unwrap();
    std::fs::set_permissions(
        home.join("bedrock_server"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    config
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "This synchronous test runs the CLI and reads or writes local fixtures"
)]
fn server_list_json_reports_each_server_for_programs() {
    let temp = tempfile::tempdir().unwrap();
    let config = fake_server(temp.path());
    let output = foundry(temp.path())
        .args(["server", "list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let servers: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        servers,
        serde_json::json!([{
            "id": config.id,
            "name": "Controller fixture",
            "port": 19140,
            "version": "1.26.60.6",
            "public": false,
            "addons": []
        }])
    );
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "This synchronous test runs the CLI and reads or writes local fixtures"
)]
fn json_run_reports_readiness_and_stops_the_server_when_input_closes() {
    let temp = tempfile::tempdir().unwrap();
    let config = fake_server(temp.path());
    let mut child = foundry(temp.path())
        .args(["server", "run", &config.id.to_string(), "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let mut events = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut next =
        || -> serde_json::Value { serde_json::from_str(&events.next().unwrap().unwrap()).unwrap() };
    assert_eq!(
        next(),
        serde_json::json!({"event": "log", "line": "[INFO] Server started."})
    );
    assert_eq!(
        next(),
        serde_json::json!({"event": "ready", "host": "127.0.0.1", "port": 19140})
    );
    // A controlling program that exits closes this pipe.
    drop(input);
    assert_eq!(next(), serde_json::json!({"event": "stopped"}));
    assert!(child.wait().unwrap().success());
    let home = server::home(&temp.path().canonicalize().unwrap(), config.id);
    assert_eq!(
        std::fs::read(home.join("stopped")).unwrap(),
        b"graceful exit"
    );
}
