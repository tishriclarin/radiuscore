use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_radiuscore"))
        .args(args)
        .output()
        .expect("run radiuscore")
}

#[test]
fn help_lists_diagnostics_and_resolves_command_topics() {
    let general = cli(&["--help"]);
    assert!(general.status.success());
    let general = String::from_utf8(general.stdout).unwrap();
    assert!(general.contains("radiuscore option82 <command>"));
    assert!(general.contains("docs/operator-cli.md"));
    assert!(!general.contains("radiuscore reload"));

    let radsec = cli(&["help", "radsec", "probe"]);
    assert!(radsec.status.success());
    assert!(String::from_utf8(radsec.stdout)
        .unwrap()
        .contains("This probe sends no RADIUS request."));

    let dhcp = cli(&["option82", "decode-dhcp", "--help"]);
    assert!(dhcp.status.success());
    assert!(String::from_utf8(dhcp.stdout)
        .unwrap()
        .contains("outer option header"));
}

#[test]
fn unknown_topic_and_command_remain_errors() {
    for (args, expected) in [
        (["help", "unknown"], "unknown help topic"),
        (["option82", "unknown"], "unknown arguments"),
    ] {
        let output = cli(&args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8(output.stderr).unwrap().contains(expected));
    }
}
