# prereg-gate

`prereg-gate` turns a preregistered experiment and its observations into a
content-addressed JSON verdict. It is deliberately conservative: a drifted
control or missing sample makes the result `inconclusive`, even when the target
measurement looks positive.

The product is a Rust library plus CLI. It is useful in lab scripts, CI jobs,
and automated research workflows that must not reinterpret acceptance criteria
after seeing results.

## What it checks

- exact, versioned TOML preregistration schema;
- at least one control and one prediction;
- finite expected values, tolerances, and evidence;
- exact `metric,value` CSV input with no unknown metrics;
- minimum sample counts and mean values inside preregistered ranges;
- SHA-256 digests of the exact specification and evidence bytes.

Verdicts are:

- `supported`: every control and prediction passes;
- `rejected`: controls pass, but at least one prediction fails;
- `inconclusive`: a control fails or any criterion lacks enough samples.

## Install

Stable Rust 1.85 or newer is required.

```console
cargo install --git https://github.com/nicholas-ruest/prereg-gate
```

## Use

```console
prereg-gate check \
  --spec examples/experiment.toml \
  --evidence examples/evidence.csv \
  --pretty
```

The included example returns exit code `0` and a `supported` receipt. A
`rejected` or `inconclusive` verdict returns exit code `2`; malformed input or
an operational failure returns `1`. Use `--output receipt.json` to write the
receipt to a file.

The TOML criteria are intentionally simple and reviewable:

```toml
schema_version = 1
experiment_id = "battery-cycle-042"

[[controls]]
metric = "reference_voltage"
expected = 3.700
tolerance = 0.010
min_samples = 2

[[predictions]]
metric = "capacity_retention_pct"
expected = 95.0
tolerance = 2.0
min_samples = 3
```

The evidence file may repeat a metric; the arithmetic mean is evaluated:

```csv
metric,value
reference_voltage,3.698
reference_voltage,3.704
capacity_retention_pct,94.1
capacity_retention_pct,95.2
capacity_retention_pct,96.0
```

## Verify

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

CI repeats these checks on stable Rust and checks the declared Rust 1.85 MSRV.

## Limits

`prereg-gate` evaluates scalar numeric metrics with symmetric absolute
tolerances and an arithmetic mean. It does not perform statistical hypothesis
testing, instrument control, outlier removal, signing, or remote evidence
collection. Input hashes make a receipt traceable, not authentic; sign and
store it in your own trusted system when authenticity matters.

## License

[MIT](LICENSE)
