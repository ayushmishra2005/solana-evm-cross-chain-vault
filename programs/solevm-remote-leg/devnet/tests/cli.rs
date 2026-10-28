//! The driver run directly, as an operator could without the scripts. Each
//! unsafe configuration must be refused before a key is read, a transaction
//! is sent or a record is written. A stub on 127.0.0.1 stands in for the RPC
//! and records every method it is asked for, so no test reaches a network.

use std::error::Error;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{Value, json};

type Res<T> = Result<T, Box<dyn Error>>;

const DEVNET: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const MAINNET: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
const TESTNET: &str = "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY";
/// None of the public clusters', as a test validator's genesis is.
const LOCAL: &str = "CyirTSae43Yhp4RvmDM6veVkqnydznFTGiCaDNSvds8i";
const SECP256R1: &str = "srremy31J5Y25FrAApwVb9kZcfXbusYMMsvTK9aWv5q";
const PRECOMPILE_IN_SVM: &str = "9ypxGLzkMxi89eDerRKXWDXe44UY2z4hBig4mDhNq5Dp";
const DISABLE_SBPF_V0_EXECUTION: &str = "TestFeature11111111111111111111111111111111";
const V2_GATES: &[&str] = &[SECP256R1, PRECOMPILE_IN_SVM];

/// A JSON-RPC endpoint serving one genesis hash and the given active feature
/// accounts. Every other account is absent and every other method an error.
struct Stub {
    url: String,
    calls: Arc<Mutex<Vec<String>>>,
}

impl Stub {
    fn start(genesis: &'static str, active: &'static [&'static str]) -> Res<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}", listener.local_addr()?);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&calls);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = serve(stream, genesis, active, &log);
            }
        });
        Ok(Self { url, calls })
    }

    fn calls(&self) -> Res<Vec<String>> {
        Ok(self.calls.lock().map_err(|_| "stub log poisoned")?.clone())
    }
}

fn serve(stream: TcpStream, genesis: &str, active: &[&str], log: &Mutex<Vec<String>>) -> Res<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut length = 0_usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse()?;
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let request: Value = serde_json::from_slice(&body)?;
    let method = request["method"].as_str().unwrap_or_default().to_owned();
    log.lock()
        .map_err(|_| "stub log poisoned")?
        .push(method.clone());
    let response = match method.as_str() {
        "getGenesisHash" => json!({ "jsonrpc": "2.0", "id": 1, "result": genesis }),
        "getAccountInfo" => {
            let key = request["params"][0].as_str().unwrap_or_default();
            // An activated feature: byte 0 set, activation slot 0.
            let value = if active.contains(&key) {
                json!({
                    "data": ["AQAAAAAAAAAA", "base64"],
                    "owner": "Feature111111111111111111111111111111111111",
                    "executable": false,
                    "lamports": 1,
                    "rentEpoch": 0,
                    "space": 9
                })
            } else {
                Value::Null
            };
            json!({ "jsonrpc": "2.0", "id": 1, "result": { "context": { "slot": 1 }, "value": value } })
        }
        _ => json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32601, "message": format!("the stub does not serve {method}") }
        }),
    };
    let body = response.to_string();
    let mut stream = stream;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;
    Ok(())
}

/// An empty directory outside the repository, used as `HOME`.
fn scratch(name: &str) -> Res<PathBuf> {
    let dir = std::env::temp_dir().join(format!("solevm-devnet-cli-{name}-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    Ok(dir.canonicalize()?)
}

fn text(path: &Path) -> Res<&str> {
    path.to_str()
        .ok_or_else(|| format!("{} is not UTF-8", path.display()).into())
}

/// Runs the driver with nothing inherited but `HOME` and `vars`.
fn driver(home: &Path, args: &[&str], vars: &[(&str, &str)]) -> Res<Output> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_solevm-devnet"));
    command.env_clear().env("HOME", home).args(args);
    for (name, value) in vars {
        command.env(name, value);
    }
    Ok(command.output()?)
}

fn refused(output: &Output, reason: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains(reason), "expected {reason:?} in: {stderr}");
    assert!(
        output.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

// V1 commands under the V2 sources

/// `command` must stop before reading settings, keys or the RPC, and leave
/// the V1 record of the same label byte-for-byte as it was.
fn v1_command_keeps_the_record(command: &str) -> Res<()> {
    let home = scratch(&format!("v1-{command}"))?;
    let v1_home = home.join(".config").join("solevm-devnet");
    let state = v1_home.join("state").join("devnet");
    fs::create_dir_all(&state)?;
    let record = state.join("evidence-devnet-1.json");
    let original = b"{\"run_label\": \"devnet-1\", \"note\": \"must stay byte-identical\"}\n";
    fs::write(&record, original)?;
    let rpc = Stub::start(DEVNET, V2_GATES)?;

    let output = driver(
        &home,
        &[command],
        &[
            ("SOLEVM_HOME", text(&v1_home)?),
            ("SOLEVM_RUN_LABEL", "devnet-1"),
            ("SOLEVM_RPC_URL", rpc.url.as_str()),
        ],
    )?;
    refused(&output, "the V1 instance is read-only");
    refused(&output, "27zReYgZvFTaUSWwob4qirN2Yxekcm32mnQDpMzeTyAE");

    assert_eq!(fs::read(&record)?, original);
    assert_eq!(fs::read_dir(&state)?.count(), 1, "nothing else was written");
    assert!(!v1_home.join("keys").exists());
    assert_eq!(
        rpc.calls()?,
        Vec::<String>::new(),
        "the RPC was never asked"
    );
    fs::remove_dir_all(&home)?;
    Ok(())
}

#[test]
fn v1_run_refuses_the_v2_build_and_keeps_the_v1_record() -> Res<()> {
    v1_command_keeps_the_record("run")
}

#[test]
fn v1_addresses_refuses_the_v2_build() -> Res<()> {
    v1_command_keeps_the_record("addresses")
}

// run-v2 invoked directly

/// Runs `run-v2` with an isolated V2 home and returns that home.
fn run_v2(name: &str, vars: &[(&str, &str)]) -> Res<(Output, PathBuf, PathBuf)> {
    let home = scratch(name)?;
    let v2_home = home.join("v2");
    let mut all = vec![("SOLEVM_V2_HOME", text(&v2_home)?.to_owned())];
    all.extend(
        vars.iter()
            .map(|(name, value)| (*name, (*value).to_owned())),
    );
    let borrowed: Vec<(&str, &str)> = all
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let output = driver(&home, &["run-v2"], &borrowed)?;
    Ok((output, home, v2_home))
}

/// Nothing was created in the V2 home: no key, no state, no record.
fn untouched(home: &Path, v2_home: &Path) -> Res<()> {
    assert!(!v2_home.exists(), "{} was created", v2_home.display());
    fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn run_v2_needs_an_explicit_url_and_cluster() -> Res<()> {
    let rpc = Stub::start(DEVNET, V2_GATES)?;
    let cases = [
        (
            "no-url",
            vec![("SOLEVM_CLUSTER", "devnet")],
            "set SOLEVM_RPC_URL explicitly",
        ),
        (
            "no-cluster",
            vec![("SOLEVM_RPC_URL", rpc.url.as_str())],
            "set SOLEVM_CLUSTER to devnet or localnet",
        ),
        (
            "mainnet-name",
            vec![
                ("SOLEVM_RPC_URL", rpc.url.as_str()),
                ("SOLEVM_CLUSTER", "mainnet-beta"),
            ],
            "set SOLEVM_CLUSTER to devnet or localnet",
        ),
        (
            "testnet-name",
            vec![
                ("SOLEVM_RPC_URL", rpc.url.as_str()),
                ("SOLEVM_CLUSTER", "testnet"),
            ],
            "set SOLEVM_CLUSTER to devnet or localnet",
        ),
    ];
    for (name, vars, reason) in cases {
        let (output, home, v2_home) = run_v2(name, &vars)?;
        refused(&output, reason);
        untouched(&home, &v2_home)?;
    }
    assert_eq!(rpc.calls()?, Vec::<String>::new());
    Ok(())
}

#[test]
fn run_v2_on_devnet_needs_the_operator_approval() -> Res<()> {
    let rpc = Stub::start(DEVNET, V2_GATES)?;
    for (name, approval) in [
        ("unapproved", None),
        ("approval-0", Some("0")),
        ("approval-yes", Some("yes")),
    ] {
        let mut vars = vec![
            ("SOLEVM_RPC_URL", rpc.url.as_str()),
            ("SOLEVM_CLUSTER", "devnet"),
        ];
        vars.extend(approval.map(|value| ("SOLEVM_V2_DEVNET_APPROVED", value)));
        let (output, home, v2_home) = run_v2(name, &vars)?;
        refused(&output, "needs SOLEVM_V2_DEVNET_APPROVED=1");
        untouched(&home, &v2_home)?;
    }
    assert_eq!(
        rpc.calls()?,
        Vec::<String>::new(),
        "refused before any RPC call"
    );
    Ok(())
}

#[test]
fn run_v2_refuses_mainnet_and_testnet_by_genesis() -> Res<()> {
    for (name, genesis, cluster) in [
        ("mainnet-as-devnet", MAINNET, "devnet"),
        ("testnet-as-devnet", TESTNET, "devnet"),
        ("mainnet-as-localnet", MAINNET, "localnet"),
        ("testnet-as-localnet", TESTNET, "localnet"),
    ] {
        let rpc = Stub::start(genesis, V2_GATES)?;
        let (output, home, v2_home) = run_v2(
            name,
            &[
                ("SOLEVM_RPC_URL", rpc.url.as_str()),
                ("SOLEVM_CLUSTER", cluster),
                ("SOLEVM_V2_DEVNET_APPROVED", "1"),
            ],
        )?;
        refused(&output, "is mainnet-beta or testnet");
        assert_eq!(rpc.calls()?, vec![String::from("getGenesisHash")]);
        untouched(&home, &v2_home)?;
    }
    Ok(())
}

#[test]
fn run_v2_refuses_an_endpoint_that_is_not_the_named_cluster() -> Res<()> {
    for (name, genesis, cluster, reason) in [
        ("local-as-devnet", LOCAL, "devnet", "which is not devnet"),
        (
            "devnet-as-localnet",
            DEVNET,
            "localnet",
            "which is not localnet",
        ),
    ] {
        let rpc = Stub::start(genesis, V2_GATES)?;
        let (output, home, v2_home) = run_v2(
            name,
            &[
                ("SOLEVM_RPC_URL", rpc.url.as_str()),
                ("SOLEVM_CLUSTER", cluster),
                ("SOLEVM_V2_DEVNET_APPROVED", "1"),
            ],
        )?;
        refused(&output, reason);
        assert_eq!(rpc.calls()?, vec![String::from("getGenesisHash")]);
        untouched(&home, &v2_home)?;
    }
    Ok(())
}

#[test]
fn run_v2_on_localnet_needs_a_local_endpoint() -> Res<()> {
    // Refused from the URL alone, so neither endpoint is ever contacted.
    for (name, url) in [
        ("public-as-localnet", "https://api.devnet.solana.com"),
        ("remote-as-localnet", "http://10.0.0.1:8899"),
    ] {
        let (output, home, v2_home) = run_v2(
            name,
            &[("SOLEVM_RPC_URL", url), ("SOLEVM_CLUSTER", "localnet")],
        )?;
        refused(
            &output,
            "a localnet endpoint must be on 127.0.0.1 or localhost",
        );
        untouched(&home, &v2_home)?;
    }
    Ok(())
}

#[test]
fn run_v2_requires_the_feature_gates_before_reading_keys() -> Res<()> {
    let cases: [(&str, &'static [&'static str], &str); 3] = [
        ("no-secp256r1", &[PRECOMPILE_IN_SVM], "SIMD-0075"),
        ("no-precompile-in-svm", &[SECP256R1], "SIMD-0159"),
        (
            "sbpf-v0-execution-disabled",
            &[SECP256R1, PRECOMPILE_IN_SVM, DISABLE_SBPF_V0_EXECUTION],
            "SIMD-0161",
        ),
    ];
    for (name, active, reason) in cases {
        let rpc = Stub::start(DEVNET, active)?;
        let (output, home, v2_home) = run_v2(
            name,
            &[
                ("SOLEVM_RPC_URL", rpc.url.as_str()),
                ("SOLEVM_CLUSTER", "devnet"),
                ("SOLEVM_V2_DEVNET_APPROVED", "1"),
            ],
        )?;
        refused(&output, reason);
        let calls = rpc.calls()?;
        assert!(
            calls
                .iter()
                .all(|method| method == "getGenesisHash" || method == "getAccountInfo"),
            "{calls:?}"
        );
        untouched(&home, &v2_home)?;
    }
    Ok(())
}

#[test]
fn run_v2_on_localnet_needs_no_devnet_approval() -> Res<()> {
    let rpc = Stub::start(LOCAL, V2_GATES)?;
    let (output, home, v2_home) = run_v2(
        "localnet-unapproved",
        &[
            ("SOLEVM_RPC_URL", rpc.url.as_str()),
            ("SOLEVM_CLUSTER", "localnet"),
        ],
    )?;
    // Past every guard, it stops at the first key the empty home lacks.
    refused(&output, "deployer.json");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("APPROVED"));
    assert_eq!(
        rpc.calls()?,
        [
            "getGenesisHash",
            "getAccountInfo",
            "getAccountInfo",
            "getAccountInfo"
        ]
    );
    untouched(&home, &v2_home)?;
    Ok(())
}

#[test]
fn run_v2_home_must_differ_from_the_v1_home() -> Res<()> {
    let home = scratch("shared-home")?;
    let shared = home.join("shared");
    let output = driver(
        &home,
        &["run-v2"],
        &[
            ("SOLEVM_HOME", text(&shared)?),
            ("SOLEVM_V2_HOME", text(&shared)?),
            ("SOLEVM_RPC_URL", "http://127.0.0.1:1"),
            ("SOLEVM_CLUSTER", "localnet"),
        ],
    )?;
    refused(&output, "must not be inside SOLEVM_HOME");
    untouched(&home, &shared)
}
