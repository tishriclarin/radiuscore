mod history;

use std::{
    env,
    num::NonZeroU32,
    path::Path,
    process::{Command, ExitCode},
};

use radiuscore::{
    admission::{Limiter, Rate},
    integrity::verify_udp_access_request,
    ipam::Prefix,
    option82::decode_hex,
    radius::Packet,
};
use serde_json::{json, Value};

fn case(tests: &mut Vec<Value>, name: &str, expected: Value, actual: Value) {
    tests.push(
        json!({"name": name, "status": if actual == expected {"passed"} else {"failed"},
        "expected": expected, "actual": actual}),
    );
}

fn outcome<T, E: std::fmt::Debug>(result: Result<T, E>) -> String {
    match result {
        Ok(_) => "Accepted".into(),
        Err(error) => format!("{error:?}"),
    }
}

fn local_cases(out: &mut Vec<Value>) {
    const REQUEST: &str =
        "0137002b000102030405060708090a0b0c0d0e0f0105626f6250121291c22f5b2039dd10eb3199558bea39";
    const KEY: &[u8] = b"isolated-fixture-radius-secret";
    let frame = decode_hex(REQUEST).unwrap();
    let verified = verify_udp_access_request(Packet::decode(&frame).unwrap(), KEY).is_ok();
    case(
        out,
        "access_request_integrity",
        json!(true),
        json!(verified),
    );
    case(
        out,
        "invalid_packet_framing",
        json!("InvalidHeader"),
        json!(outcome(Packet::decode(&[0u8; 4]))),
    );

    let mut malformed = frame.clone();
    malformed[21] = 1;
    let result = outcome(verify_udp_access_request(
        Packet::decode(&malformed).unwrap(),
        KEY,
    ));
    case(
        out,
        "invalid_attribute_length",
        json!("MalformedAttribute"),
        json!(result),
    );

    let mut bad_value = frame;
    bad_value[24] ^= 1;
    let result = outcome(verify_udp_access_request(
        Packet::decode(&bad_value).unwrap(),
        KEY,
    ));
    case(
        out,
        "invalid_authenticated_attribute_value",
        json!("InvalidAuthenticator"),
        json!(result),
    );

    case(
        out,
        "invalid_ip_prefix_value",
        json!("HostBitsSet"),
        json!(outcome("192.0.2.3/24".parse::<Prefix>())),
    );

    let rate = |burst| Rate {
        burst: NonZeroU32::new(burst).unwrap(),
        per_second: NonZeroU32::new(1).unwrap(),
    };
    let known_nas = 7;
    let first = [1; 32];
    let second = [2; 32];
    let third = [3; 32];
    let mut limits = Limiter::new([known_nas], rate(5), rate(1), 2, 0);
    let expected = [
        "Unauthenticated",
        "Unauthenticated",
        "Allowed",
        "SessionLimited",
        "Allowed",
        "Capacity",
        "Capacity",
        "NasLimited",
        "Allowed",
    ];
    // Logical admission checks on a fixed clock. No packet is transmitted.
    let actual: Vec<_> = [
        (None, first, 0),
        (Some(99), first, 0),
        (Some(known_nas), first, 0),
        (Some(known_nas), first, 0),
        (Some(known_nas), second, 0),
        (Some(known_nas), third, 0),
        (Some(known_nas), third, 0),
        (Some(known_nas), third, 0),
        (Some(known_nas), first, 1000),
    ]
    .into_iter()
    .map(|(nas, key, at)| format!("{:?}", limits.check(nas, Some(key), at)))
    .collect();
    case(
        out,
        "unauthenticated_nas_and_verified_budgets",
        json!(expected),
        json!(actual),
    );
    // A refill restores the session budget. An idle slot can then be reclaimed.
    limits.prune_idle(1000);
    let decision = limits.check(Some(known_nas), Some(third), 2000);
    case(
        out,
        "idle_rate_state_reclaimed",
        json!("Allowed"),
        json!(format!("{decision:?}")),
    );
}

fn database_cases(seed: u32, out: &mut Vec<Value>) -> Result<(), String> {
    if env::var_os("PGDATABASE")
        .filter(|name| !name.is_empty())
        .is_none()
    {
        return Err(
            "set PGDATABASE to a disposable database with the RadiusCore migrations applied".into(),
        );
    }
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("simulation/lifecycle.sql");
    let result = Command::new("psql")
        .args([
            "-XqAt",
            "-v",
            "ON_ERROR_STOP=1",
            "-v",
            &format!("seed={seed}"),
            "-f",
        ])
        .arg(file)
        .env("PGCONNECT_TIMEOUT", "5")
        .env(
            "PGOPTIONS",
            "-c statement_timeout=15000 -c lock_timeout=5000",
        )
        .output()
        .map_err(|err| format!("could not run psql: {err}"))?;
    if !result.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&result.stderr).trim());
        return Err("PostgreSQL simulation failed; see stderr".into());
    }
    let allowed = [
        "ordered_session_10",
        "ordered_session_11",
        "ordered_session_12",
        "out_of_order_and_duplicate_accounting",
        "dhcp_t1_single_slot_renewal",
        "abrupt_missing_stop_detected",
        "confirmed_disconnect_reconciles_leftovers",
        "expired_lease_reconciles_before_reuse",
        "deactivated_account_denies_access",
    ];
    let mut seen = std::collections::HashSet::new();
    for line in String::from_utf8_lossy(&result.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let row: Value = serde_json::from_str(line).map_err(|_| "invalid PostgreSQL result")?;
        let name = row["name"].as_str().ok_or("missing PostgreSQL test name")?;
        if !allowed.contains(&name) || !seen.insert(name.to_owned()) {
            return Err(format!("unexpected or duplicate PostgreSQL case: {name}"));
        }
        case(out, name, row["expected"].clone(), row["actual"].clone());
    }
    if seen.len() != allowed.len() {
        return Err("one or more PostgreSQL scenarios returned no result".into());
    }
    Ok(())
}

fn main() -> ExitCode {
    match history::execute() {
        Ok(code) => code,
        Err(error) => {
            println!(
                "{}",
                json!({"status":"failed","summary":{"passed":0,"failed":0},"error":error})
            );
            ExitCode::FAILURE
        }
    }
}
