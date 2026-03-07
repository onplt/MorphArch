# CI/CD Integration

Integrate MorphArch into your continuous integration pipeline to enforce architectural standards on every Pull Request.

## GitHub Actions Example

You can use MorphArch to fail a build if the architectural health score drops below a certain threshold (e.g., 80/100).

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
          fetch-depth: 0 # Full history is required for scan

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Install MorphArch
        run: cargo install morpharch

      - name: Run Architecture Analysis
        run: |
          # Scan the repository
          morpharch scan . --max-commits 1
          
          # Analyze the HEAD commit and extract the score
          SCORE=$(morpharch analyze --json | jq '.total')
          
          echo "Architectural Health Score: $SCORE"
          
          # Fail if score is below 80
          if [ "$SCORE" -lt 80 ]; then
            echo "Architecture health is too low ($SCORE)! Please fix circular dependencies or violations."
            exit 1
          fi
```

---

## Why run in CI/CD?

- **Prevent Architecture Decay**: Detect circular dependencies as soon as they are introduced.
- **Enforce Layer Boundaries**: Ensure that `shared` packages never depend on `app` code.
- **Track Drift**: Monitor how architectural debt changes over time in your repository history.

:::tip Outputting JSON
The `morpharch analyze --json` flag is designed for easy integration with tools like `jq` in CI/CD environments.
:::
