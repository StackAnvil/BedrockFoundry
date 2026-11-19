//! Installs an arbitrary BDS distribution archive into a sandboxed server
//! instance. No file may be written outside the instance directory, and the
//! archive must not supply the instance's world, backup, or key directories.
#![no_main]

use bedrock_foundry::server::{ServerConfig, home, import_distribution};
use bedrock_foundry_fuzz::Sandbox;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let sandbox = Sandbox::new();
    let mut config = ServerConfig::new("Fuzz".into(), 19132).expect("valid server configuration");
    let instance = home(&sandbox.dest, config.id);
    let result = import_distribution(&sandbox.dest, &mut config, data, "1.21.0.1".into());
    sandbox.assert_contained(std::slice::from_ref(&instance));
    if result.is_ok() {
        for managed in ["worlds", "backups", "keys"] {
            assert!(
                !instance.join(managed).exists(),
                "the distribution supplied the managed {managed} directory"
            );
        }
    }
});
