# CI/CD Integration

Integrate MorphArch into your CI pipeline to capture architectural drift on
every pull request.

## GitHub Actions Example

This example runs a shallow scan, prints a human-readable report for `HEAD`, and
fails the build when the reported health drops below a threshold.

MorphArch currently emits human-readable CLI output, so the example extracts the
health percentage from `morpharch analyze`.

```yaml
# .github/workflows/architecture.yml
name: Architecture Health

on: [pull_request]

jobs:
  check-health:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Install MorphArch
        run: |
          curl -fsSL https://raw.githubusercontent.com/onplt/morpharch/main/install.sh | sh

      - name: Run Architecture Analysis
        run: |
          # Scan only the current commit for CI feedback
          morpharch scan . --max-commits 1

          # Analyze HEAD and capture the report
          REPORT="$(morpharch analyze HEAD --path .)"
          printf '%s\n' "$REPORT"

          # Extract the "Health: NN%" line from the report
          SCORE="$(printf '%s\n' "$REPORT" | sed -n 's/^     Health: \([0-9][0-9]*\)%.*/\1/p' | head -n 1)"

          if [ -z "$SCORE" ]; then
            echo "Failed to extract health score from MorphArch output."
            exit 1
          fi

          echo "Architectural Health Score: $SCORE"

          # Fail if score is below 80
          if [ "$SCORE" -lt 80 ]; then
            echo "Architecture health is too low ($SCORE)! Please review cycles, coupling, or boundary drift."
            exit 1
          fi
```

### Alternative Install Methods for CI

The shell script installer is the fastest option since it downloads a pre-built
binary. If you prefer other methods:

| Method | Command | Notes |
|--------|---------|-------|
| **Shell script** (Recommended) | `curl -fsSL .../install.sh \| sh` | Pre-built binary, fastest |
| **cargo-binstall** | `cargo binstall morpharch --no-confirm` | Pre-built binary via cargo |
| **cargo install** | `cargo install morpharch` | Compiles from source, slowest |

---

## Why run in CI/CD?

- **Prevent architecture decay**: detect circular dependencies as soon as they are introduced.
- **Enforce layer boundaries**: ensure that `shared` packages never depend on `app` code.
- **Track drift**: monitor how architectural debt changes over time in your repository history.

:::tip Keep CI scans shallow
For pull requests, `--max-commits 1` or another small number is usually enough.
Use deeper scans for scheduled jobs or local investigation.
:::
