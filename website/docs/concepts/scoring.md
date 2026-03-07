# Architectural Health Scoring

The **Scoring Engine** is the core of MorphArch. It translates complex graph relationships into a single, actionable **0-100 Health Score**.

## The Philosophy: Debt vs. Health

MorphArch views architecture through the lens of **Technical Debt**. We start with a perfect score of **100** and subtract points for structural flaws that hinder maintainability.

---

## 1. Cyclic Dependencies (-25 pts)
Cycles are the most severe architectural flaw. They break modularity, making it impossible to test or deploy packages in isolation.

- **Detection**: Uses the **Kosaraju's Algorithm** to find **Strongly Connected Components (SCC)**.
- **Penalty**: -25 points per group of packages that form a cycle.
- **Example**:
  - `pkg-A` imports `pkg-B`
  - `pkg-B` imports `pkg-C`
  - `pkg-C` imports `pkg-A` **(Cycle Detected!)**

:::danger Why it matters
Cycles lead to "Spaghetti Code" where changing one line in a library might break an entire unrelated application.
:::

---

## 2. Boundary Violations (-15 pts)
Boundaries define the "flow" of your architecture. High-level modules (apps) should depend on low-level modules (libs), never the other way around.

- **Detection**: Compares extracted AST imports against rules defined in your `morpharch.toml`.
- **Penalty**: -15 points per unique violation edge.
- **Example Violation**:
  ```toml
  # morpharch.toml rules
  rules = [["libs/", "apps/"]] # libs should NOT depend on apps
  ```
  ```typescript
  // apps/web/utils.ts
  export const VERSION = '1.0';

  // libs/core/logger.ts
  import { VERSION } from '../../apps/web/utils'; // Error: Boundary Violation!
  ```

---

## 3. Coupling Density (-5 pts)
Large systems are naturally complex, but excessive connections lead to fragility.

- **Metric**: `Edge Count / Node Count`.
- **Threshold**: MorphArch uses a base threshold of **3.5**.
- **Penalty**: -5 points for every 1.0 unit above the threshold.
- **Scale Grace**: Projects with fewer than 10 nodes are exempt from density penalties to allow for early-stage rapid prototyping.

---

## How to Improve Your Score

1.  **Break Cycles**: Use dependency inversion or extract shared logic into a new, lower-level package.
2.  **Define Boundaries**: Explicitly list your layer rules in `morpharch.toml`.
3.  **Refactor "God Packages"**: If a package has a high instability index, it likely has too many responsibilities. Split it.
