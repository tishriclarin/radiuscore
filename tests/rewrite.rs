use std::cell::Cell;
use std::sync::Arc;

use radiuscore::rewrite::{Error, Received, RewritePlan, RuleSpec, TargetSpec, Transform};

fn plan() -> RewritePlan<3> {
    let circuit_rules = [RuleSpec {
        guard: Some((0, r"(?i)^00:11:22(?::[0-9a-f]{2}){3}$")),
        pattern: "CID=([A-Fa-f0-9]+);END",
        capture: 1,
        prefix: b"0x",
        suffix: b"",
        transform: Transform::HexUpper,
    }];
    let remote_rules = [RuleSpec {
        guard: None,
        pattern: r"REMOTE=(.+)",
        capture: 1,
        prefix: b"",
        suffix: b"",
        transform: Transform::LowerAscii,
    }];
    RewritePlan::compile(&[
        TargetSpec {
            slot: 1,
            required: true,
            min_octets: 1,
            max_octets: 63,
            rules: &circuit_rules,
        },
        TargetSpec {
            slot: 2,
            required: true,
            min_octets: 1,
            max_octets: 63,
            rules: &remote_rules,
        },
    ])
    .unwrap()
}

fn received<'a>(caller: Option<&'a [u8]>, circuit: &'a [u8], remote: &'a [u8]) -> Received<'a, 3> {
    let mut packet = Received::new();
    packet.record(1, circuit).unwrap();
    packet.record(2, remote).unwrap();
    if let Some(caller) = caller {
        packet.record(0, caller).unwrap();
    }
    packet
}

#[test]
fn failed_required_rewrites_never_invoke_lookup() {
    let plan = plan();
    let caller = Some(b"00:11:22:aa:bb:cc".as_slice());
    let mut worker = plan.worker();
    let calls = Cell::new(0);
    let packet = received(caller, b"CID=00a1;END", b"REMOTE=ABC");
    worker
        .then_lookup(&packet, |working| {
            assert_eq!(working.attribute(1).unwrap(), Some(b"0x00A1".as_slice()));
            assert_eq!(working.attribute(2).unwrap(), Some(b"abc".as_slice()));
            calls.set(calls.get() + 1);
        })
        .unwrap();
    for (packet, error) in [
        (
            received(None, b"CID=00a1;END", b"REMOTE=ABC"),
            Error::RequiredOutputMissing,
        ),
        (
            received(caller, b"CID=abc;END", b"REMOTE=ABC"),
            Error::InvalidOutput,
        ),
        (
            received(caller, b"CID=00a1;END", b"BROKEN"),
            Error::RequiredOutputMissing,
        ),
    ] {
        assert_eq!(
            worker.then_lookup(&packet, |_| calls.set(calls.get() + 1)),
            Err(error)
        );
    }
    let mut packet = received(caller, b"CID=00a1;END", b"REMOTE=ABC");
    packet.record(0, b"00:11:22:aa:bb:cc").unwrap();
    assert_eq!(
        worker.then_lookup(&packet, |_| calls.set(calls.get() + 1)),
        Err(Error::DuplicateAttribute)
    );
    assert_eq!(calls.get(), 1);
}

#[test]
fn a_shared_plan_works_across_workers_with_separate_scratch() {
    let plan = Arc::new(plan());
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let plan = Arc::clone(&plan);
            std::thread::spawn(move || {
                let mut worker = plan.worker();
                for _ in 0..300 {
                    let packet =
                        received(Some(b"00:11:22:aa:bb:cc"), b"CID=00a1;END", b"REMOTE=LAB");
                    worker
                        .then_lookup(&packet, |working| {
                            assert_eq!(working.attribute(1).unwrap(), Some(b"0x00A1".as_slice()));
                        })
                        .unwrap();
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
}
