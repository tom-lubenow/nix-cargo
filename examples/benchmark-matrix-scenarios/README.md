# Benchmark matrix scenario files

Scenario files are tab-separated.

Supported row formats:

- `name<TAB>mutation-file`
- `name<TAB>workspace<TAB>target-crate<TAB>mutation-file`

`workspace` can be absolute or repository-relative.

Example:

```text
leaf_edit	examples/incremental-workspace-large	app	crates/leaf_a/src/lib.rs
```

Included scenario files:

- `large.tsv`: single-workspace large DAG scenarios.
- `repo.tsv`: repository-backed mixed scenarios (small + large fixtures).
