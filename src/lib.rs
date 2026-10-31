//! Deterministic evaluation of experimental evidence against a preregistration.

use std::collections::{BTreeMap, BTreeSet};

use csv::StringRecord;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const SUPPORTED_SCHEMA_VERSION: u8 = 1;

/// Errors that prevent a trustworthy verdict from being produced.
#[derive(Debug, Error)]
pub enum GateError {
    #[error("preregistration is not valid UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("could not parse preregistration TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("could not parse evidence CSV: {0}")]
    Csv(#[from] csv::Error),
    #[error("invalid preregistration: {0}")]
    InvalidSpec(String),
    #[error("invalid evidence: {0}")]
    InvalidEvidence(String),
}

/// A preregistered experiment contract.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentSpec {
    pub schema_version: u8,
    pub experiment_id: String,
    pub controls: Vec<Criterion>,
    pub predictions: Vec<Criterion>,
}

/// An expected metric range and minimum sample count.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub metric: String,
    pub expected: f64,
    pub tolerance: f64,
    #[serde(default = "default_min_samples")]
    pub min_samples: usize,
}

const fn default_min_samples() -> usize {
    1
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceRow {
    metric: String,
    value: f64,
}

/// Final decision produced by the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Supported,
    Rejected,
    Inconclusive,
}

impl Verdict {
    /// Process exit code used by the CLI.
    #[must_use]
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Supported => 0,
            Self::Rejected | Self::Inconclusive => 2,
        }
    }
}

/// Whether one criterion passed, failed, or lacked enough evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    Passed,
    Failed,
    Missing,
}

/// Kind of preregistered criterion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionKind {
    Control,
    Prediction,
}

/// Reproducible result for one preregistered criterion.
#[derive(Debug, Clone, Serialize)]
pub struct CheckReceipt {
    pub kind: CriterionKind,
    pub metric: String,
    pub outcome: CheckOutcome,
    pub samples: usize,
    pub min_samples: usize,
    pub observed_mean: Option<f64>,
    pub expected: f64,
    pub tolerance: f64,
    pub accepted_min: f64,
    pub accepted_max: f64,
    pub reason: String,
}

/// Summary counts for quick machine checks.
#[derive(Debug, Clone, Serialize)]
pub struct ReceiptSummary {
    pub controls_passed: usize,
    pub controls_failed_or_missing: usize,
    pub predictions_passed: usize,
    pub predictions_failed: usize,
    pub predictions_missing: usize,
}

/// Content-addressed decision receipt.
#[derive(Debug, Clone, Serialize)]
pub struct Receipt {
    pub schema_version: u8,
    pub experiment_id: String,
    pub verdict: Verdict,
    pub spec_sha256: String,
    pub evidence_sha256: String,
    pub summary: ReceiptSummary,
    pub checks: Vec<CheckReceipt>,
}

/// Evaluate raw TOML and CSV bytes, preserving hashes of the exact inputs.
pub fn evaluate(spec_bytes: &[u8], evidence_bytes: &[u8]) -> Result<Receipt, GateError> {
    let spec: ExperimentSpec = toml::from_str(std::str::from_utf8(spec_bytes)?)?;
    validate_spec(&spec)?;
    let evidence = parse_evidence(evidence_bytes, &spec)?;

    let mut checks = Vec::with_capacity(spec.controls.len() + spec.predictions.len());
    for criterion in &spec.controls {
        checks.push(evaluate_criterion(
            CriterionKind::Control,
            criterion,
            evidence.get(&criterion.metric),
        ));
    }
    for criterion in &spec.predictions {
        checks.push(evaluate_criterion(
            CriterionKind::Prediction,
            criterion,
            evidence.get(&criterion.metric),
        ));
    }

    let summary = summarize(&checks);
    let verdict = if summary.controls_failed_or_missing > 0 || summary.predictions_missing > 0 {
        Verdict::Inconclusive
    } else if summary.predictions_failed > 0 {
        Verdict::Rejected
    } else {
        Verdict::Supported
    };

    Ok(Receipt {
        schema_version: SUPPORTED_SCHEMA_VERSION,
        experiment_id: spec.experiment_id,
        verdict,
        spec_sha256: sha256_hex(spec_bytes),
        evidence_sha256: sha256_hex(evidence_bytes),
        summary,
        checks,
    })
}

fn validate_spec(spec: &ExperimentSpec) -> Result<(), GateError> {
    if spec.schema_version != SUPPORTED_SCHEMA_VERSION {
        return Err(GateError::InvalidSpec(format!(
            "schema_version must be {SUPPORTED_SCHEMA_VERSION}, got {}",
            spec.schema_version
        )));
    }
    if spec.experiment_id.trim().is_empty() {
        return Err(GateError::InvalidSpec(
            "experiment_id must not be empty".to_owned(),
        ));
    }
    if spec.controls.is_empty() {
        return Err(GateError::InvalidSpec(
            "at least one control is required".to_owned(),
        ));
    }
    if spec.predictions.is_empty() {
        return Err(GateError::InvalidSpec(
            "at least one prediction is required".to_owned(),
        ));
    }

    let mut metrics = BTreeSet::new();
    for criterion in spec.controls.iter().chain(&spec.predictions) {
        if criterion.metric.trim().is_empty() {
            return Err(GateError::InvalidSpec(
                "criterion metric must not be empty".to_owned(),
            ));
        }
        if !metrics.insert(criterion.metric.as_str()) {
            return Err(GateError::InvalidSpec(format!(
                "duplicate criterion metric {:?}",
                criterion.metric
            )));
        }
        if !criterion.expected.is_finite() {
            return Err(GateError::InvalidSpec(format!(
                "expected value for {:?} must be finite",
                criterion.metric
            )));
        }
        if !criterion.tolerance.is_finite() || criterion.tolerance < 0.0 {
            return Err(GateError::InvalidSpec(format!(
                "tolerance for {:?} must be finite and non-negative",
                criterion.metric
            )));
        }
        if criterion.min_samples == 0 {
            return Err(GateError::InvalidSpec(format!(
                "min_samples for {:?} must be at least 1",
                criterion.metric
            )));
        }
    }
    Ok(())
}

fn parse_evidence(
    evidence_bytes: &[u8],
    spec: &ExperimentSpec,
) -> Result<BTreeMap<String, Vec<f64>>, GateError> {
    let known_metrics: BTreeSet<&str> = spec
        .controls
        .iter()
        .chain(&spec.predictions)
        .map(|criterion| criterion.metric.as_str())
        .collect();
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(evidence_bytes);
    let expected_headers = StringRecord::from(vec!["metric", "value"]);
    if reader.headers()? != &expected_headers {
        return Err(GateError::InvalidEvidence(
            "CSV header must be exactly: metric,value".to_owned(),
        ));
    }

    let mut evidence = BTreeMap::<String, Vec<f64>>::new();
    for (index, row) in reader.deserialize::<EvidenceRow>().enumerate() {
        let row = row?;
        if !known_metrics.contains(row.metric.as_str()) {
            return Err(GateError::InvalidEvidence(format!(
                "row {} uses unknown metric {:?}",
                index + 2,
                row.metric
            )));
        }
        if !row.value.is_finite() {
            return Err(GateError::InvalidEvidence(format!(
                "row {} value must be finite",
                index + 2
            )));
        }
        evidence.entry(row.metric).or_default().push(row.value);
    }
    Ok(evidence)
}

fn evaluate_criterion(
    kind: CriterionKind,
    criterion: &Criterion,
    observations: Option<&Vec<f64>>,
) -> CheckReceipt {
    let samples = observations.map_or(0, Vec::len);
    let accepted_min = criterion.expected - criterion.tolerance;
    let accepted_max = criterion.expected + criterion.tolerance;
    let observed_mean = observations
        .filter(|values| !values.is_empty())
        .map(|values| values.iter().sum::<f64>() / values.len() as f64);

    let (outcome, reason) = if samples < criterion.min_samples {
        (
            CheckOutcome::Missing,
            format!(
                "requires at least {} sample(s), received {samples}",
                criterion.min_samples
            ),
        )
    } else if observed_mean.is_some_and(|mean| (accepted_min..=accepted_max).contains(&mean)) {
        (
            CheckOutcome::Passed,
            "observed mean is inside the preregistered range".to_owned(),
        )
    } else {
        (
            CheckOutcome::Failed,
            "observed mean is outside the preregistered range".to_owned(),
        )
    };

    CheckReceipt {
        kind,
        metric: criterion.metric.clone(),
        outcome,
        samples,
        min_samples: criterion.min_samples,
        observed_mean,
        expected: criterion.expected,
        tolerance: criterion.tolerance,
        accepted_min,
        accepted_max,
        reason,
    }
}

fn summarize(checks: &[CheckReceipt]) -> ReceiptSummary {
    let mut summary = ReceiptSummary {
        controls_passed: 0,
        controls_failed_or_missing: 0,
        predictions_passed: 0,
        predictions_failed: 0,
        predictions_missing: 0,
    };
    for check in checks {
        match (check.kind, check.outcome) {
            (CriterionKind::Control, CheckOutcome::Passed) => summary.controls_passed += 1,
            (CriterionKind::Control, CheckOutcome::Failed | CheckOutcome::Missing) => {
                summary.controls_failed_or_missing += 1;
            }
            (CriterionKind::Prediction, CheckOutcome::Passed) => summary.predictions_passed += 1,
            (CriterionKind::Prediction, CheckOutcome::Failed) => summary.predictions_failed += 1,
            (CriterionKind::Prediction, CheckOutcome::Missing) => summary.predictions_missing += 1,
        }
    }
    summary
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = r#"
schema_version = 1
experiment_id = "spin-response-v1"

[[controls]]
metric = "reference_signal"
expected = 100.0
tolerance = 2.0
min_samples = 2

[[predictions]]
metric = "target_signal"
expected = 42.0
tolerance = 1.0
"#;

    #[test]
    fn supports_when_controls_and_predictions_pass() {
        let evidence =
            b"metric,value\nreference_signal,99\nreference_signal,101\ntarget_signal,42.5\n";
        let receipt = evaluate(SPEC.as_bytes(), evidence).expect("valid experiment");

        assert_eq!(receipt.verdict, Verdict::Supported);
        assert_eq!(receipt.summary.controls_passed, 1);
        assert_eq!(receipt.summary.predictions_passed, 1);
        assert_eq!(receipt.spec_sha256.len(), 64);
    }

    #[test]
    fn control_drift_makes_positive_target_inconclusive() {
        let evidence =
            b"metric,value\nreference_signal,90\nreference_signal,91\ntarget_signal,42\n";
        let receipt = evaluate(SPEC.as_bytes(), evidence).expect("valid experiment");

        assert_eq!(receipt.verdict, Verdict::Inconclusive);
        assert_eq!(receipt.summary.controls_failed_or_missing, 1);
        assert_eq!(receipt.summary.predictions_passed, 1);
    }

    #[test]
    fn contradiction_rejects_when_controls_are_clean() {
        let evidence =
            b"metric,value\nreference_signal,99\nreference_signal,100\ntarget_signal,38\n";
        let receipt = evaluate(SPEC.as_bytes(), evidence).expect("valid experiment");

        assert_eq!(receipt.verdict, Verdict::Rejected);
        assert_eq!(receipt.summary.predictions_failed, 1);
    }

    #[test]
    fn missing_samples_are_inconclusive() {
        let evidence = b"metric,value\nreference_signal,100\ntarget_signal,42\n";
        let receipt = evaluate(SPEC.as_bytes(), evidence).expect("valid experiment");

        assert_eq!(receipt.verdict, Verdict::Inconclusive);
        assert_eq!(receipt.checks[0].outcome, CheckOutcome::Missing);
    }

    #[test]
    fn unknown_metric_fails_closed() {
        let evidence = b"metric,value\nreference_signal,100\nother,42\n";
        let error = evaluate(SPEC.as_bytes(), evidence).expect_err("unknown metric must fail");

        assert!(error.to_string().contains("unknown metric"));
    }

    #[test]
    fn malformed_spec_fails_closed() {
        let bad_spec = SPEC.replace("tolerance = 2.0", "tolerance = -1.0");
        let error = evaluate(bad_spec.as_bytes(), b"metric,value\n")
            .expect_err("negative tolerance must fail");

        assert!(error.to_string().contains("non-negative"));
    }
}
