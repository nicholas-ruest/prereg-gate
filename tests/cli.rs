use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;

const SPEC: &str = r#"
schema_version = 1
experiment_id = "demo"

[[controls]]
metric = "calibration"
expected = 10.0
tolerance = 0.5

[[predictions]]
metric = "response"
expected = 20.0
tolerance = 1.0
"#;

#[test]
fn supported_run_prints_receipt_and_exits_zero() {
    let directory = tempfile::tempdir().expect("tempdir");
    let spec = directory.path().join("experiment.toml");
    let evidence = directory.path().join("evidence.csv");
    fs::write(&spec, SPEC).expect("write spec");
    fs::write(&evidence, "metric,value\ncalibration,10\nresponse,20.5\n").expect("write evidence");

    Command::cargo_bin("prereg-gate")
        .expect("binary")
        .args(["check", "--spec"])
        .arg(spec)
        .arg("--evidence")
        .arg(evidence)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"verdict\":\"supported\""));
}

#[test]
fn drifted_control_exits_two_with_inconclusive_receipt() {
    let directory = tempfile::tempdir().expect("tempdir");
    let spec = directory.path().join("experiment.toml");
    let evidence = directory.path().join("evidence.csv");
    fs::write(&spec, SPEC).expect("write spec");
    fs::write(&evidence, "metric,value\ncalibration,8\nresponse,20\n").expect("write evidence");

    Command::cargo_bin("prereg-gate")
        .expect("binary")
        .args(["check", "--spec"])
        .arg(spec)
        .arg("--evidence")
        .arg(evidence)
        .assert()
        .code(2)
        .stdout(predicate::str::contains("\"verdict\":\"inconclusive\""));
}

#[test]
fn malformed_evidence_exits_one_without_receipt() {
    let directory = tempfile::tempdir().expect("tempdir");
    let spec = directory.path().join("experiment.toml");
    let evidence = directory.path().join("evidence.csv");
    fs::write(&spec, SPEC).expect("write spec");
    fs::write(&evidence, "name,reading\ncalibration,10\n").expect("write evidence");

    Command::cargo_bin("prereg-gate")
        .expect("binary")
        .args(["check", "--spec"])
        .arg(spec)
        .arg("--evidence")
        .arg(evidence)
        .assert()
        .failure()
        .stderr(predicate::str::contains("CSV header must be exactly"));
}
