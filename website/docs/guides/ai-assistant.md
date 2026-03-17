# AI Architecture Assistant

MorphArch includes a built-in AI assistant that answers natural language
questions about your codebase's architectural health, dependencies, risk
areas, and refactoring opportunities.

The assistant works with any OpenAI-compatible API endpoint: OpenAI, Ollama,
LM Studio, vLLM, or any other provider that follows the chat completions
format.

---

## Setup

### OpenAI (default)

Set your API key as an environment variable:

```bash
export OPENAI_API_KEY="sk-..."
```

No config file changes are needed. MorphArch defaults to `gpt-4o-mini` via
the OpenAI API.

### Local models (Ollama, LM Studio)

Add an `[ai]` section to `morpharch.toml`:

```toml
[ai]
provider = "ollama"
api_key_env = ""
model = "llama3.1:8b"
endpoint = "http://localhost:11434/v1/chat/completions"
stream = true
```

For LM Studio:

```toml
[ai]
provider = "lmstudio"
api_key_env = ""
model = "local-model"
endpoint = "http://localhost:1234/v1/chat/completions"
stream = true
```

### Other providers

Any endpoint that accepts the OpenAI chat completions format works. Set the
`endpoint`, `model`, and `api_key_env` accordingly.

---

## Configuration Reference

All `[ai]` fields and their defaults:

```toml
[ai]
provider = "openai"          # Provider name (for display only)
api_key_env = "OPENAI_API_KEY"  # Environment variable holding the API key
model = "gpt-4o-mini"        # Model name sent to the endpoint
endpoint = "https://api.openai.com/v1/chat/completions"
stream = true                # SSE streaming (real-time token display)
max_tokens = 4096            # Maximum tokens in AI response
temperature = 0.3            # Response creativity (0.0 = deterministic, 1.0 = creative)
max_context_tokens = 12000   # Token budget for architecture context data
```

| Field | Default | Description |
|-------|---------|-------------|
| `provider` | `"openai"` | Display name for the provider (shown in `/model` output) |
| `api_key_env` | `"OPENAI_API_KEY"` | Name of the environment variable containing the API key. Leave empty for local models that do not require authentication. |
| `model` | `"gpt-4o-mini"` | Model identifier sent in the API request |
| `endpoint` | `"https://api.openai.com/v1/chat/completions"` | Full URL of the chat completions endpoint |
| `stream` | `true` | Enable Server-Sent Events (SSE) streaming. Set to `false` if your endpoint does not support streaming. When enabled, tokens appear in the TUI as they arrive. |
| `max_tokens` | `4096` | Maximum number of tokens the model is allowed to generate per response. Increase for longer answers; decrease to save cost. |
| `temperature` | `0.3` | Controls response randomness. Lower values produce more consistent, focused answers. Higher values allow more creative analysis. |
| `max_context_tokens` | `12000` | Token budget for the architecture context data sent alongside each question. MorphArch automatically compresses the context to fit within this budget using a 4-tier priority system. |

:::tip Choosing max_context_tokens
For models with large context windows (128K+), you can increase this to 20000
or more for richer analysis. For smaller models (4K-8K context), lower it to
4000-6000 to leave room for the system prompt and response.
:::

---

## Using the Assistant

### Opening the panel

Press `a` in the TUI to toggle the AI assistant panel. The panel appears at
the bottom of the screen and can be resized by dragging its top border.

### Asking questions

Type your question in the input field and press `Enter`. The assistant
processes your question against the current architecture snapshot and streams
the response in real-time.

### Context awareness

The assistant automatically knows:

- **What you are viewing**: overview, cluster detail, or module inspect
- **Which module or cluster is selected**: focused queries get enriched context
- **Current timeline position**: which commit is displayed

When you inspect a specific module, the assistant receives additional
structured data about that module:

- Full lists of all inbound and outbound edges with weights
- Blast score and keystone status
- Cycle membership and cycle partners
- Churn count (how many commits touched the module)
- Bus factor (unique contributor count and top contributor)
- Function count, type count, and cyclomatic complexity
- Cluster membership

### Conversation history

The assistant maintains conversation context across multiple questions. Up to
3 previous exchanges are included as history, allowing follow-up questions
like "tell me more about that" or "what about the second module you
mentioned?".

### Input features

- `Up` / `Down`: recall previous questions from input history
- `Ctrl+A` / `Home`: jump to start of input
- `Ctrl+E` / `End`: jump to end of input
- `Ctrl+W`: delete word before cursor
- `Ctrl+R`: retry last query
- `Esc` while streaming: cancel the current response
- `Esc` while idle: close the AI panel

---

## Slash Commands

| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/model` | Display current AI configuration (provider, model, endpoint, streaming, max tokens, temperature) |
| `/diff` | Compare architecture with the previous commit |
| `/diff HEAD~N` or `/diff N` | Compare architecture with N commits ago |
| `/clear` | Clear conversation history |
| `/history` | Show conversation statistics (total entries, completed, history buffer) |
| `/export` | Export conversation to `morpharch-ai-conversation.txt` |

### The /diff command

`/diff` computes a structured comparison between two snapshots and asks the
AI to analyze the architectural changes:

```
/diff          → compare with previous commit
/diff HEAD~5   → compare with 5 commits ago
/diff 10       → compare with 10 commits ago
```

The diff includes:
- Added and removed modules
- Added, removed, and weight-changed edges
- Drift component deltas (cycle debt, layering debt, hub debt, etc.)

---

## Context-Aware Suggestions

The assistant generates up to 5 suggested questions based on what you are
currently viewing:

**Overview suggestions:**
- "Which cluster has the highest coupling?"
- "What causes the most cycle debt?"

**Module/cluster suggestions:**
- "Why is \{module\} fragile?"
- "What breaks if \{module\} changes?"
- "How to reduce \{module\}'s coupling?"

**Conditional suggestions:**
- "How can I break the circular dependencies?" (when cycle debt > 30%)
- "What are the top 3 things to improve health?" (when health < 50%)
- "What caused the recent health decline?" (when trend is declining)

**Timeline suggestions:**
- "What changed in this commit?" (when viewing a non-latest commit)

**Post-response suggestions:**
After the AI mentions specific modules, navigation suggestions appear:
- "→ Inspect std"
- "→ Inspect deno_core"

Press `Tab` to select a suggestion, then `Enter` to submit it. Navigation
suggestions (starting with →) take you directly to the module inspect view.

---

## Module Highlighting

Module names that appear in AI responses are automatically highlighted with a
distinct color (lavender bold). This visual distinction helps you immediately
identify which modules the AI is discussing without scanning for names
manually.

The highlighting uses word-boundary matching and longest-match-first ordering
to avoid partial matches like highlighting "std" inside "stdout".

---

## Architecture Context

The assistant receives a comprehensive JSON snapshot of your codebase's
architecture alongside each question. This includes:

### Core metrics
- **Graph summary**: node/edge counts, top hub modules
- **Drift score**: 0-100 debt score with 6 sub-component breakdowns
- **Module distribution**: total modules, brittle/stable counts, instability percentiles

### Structural analysis
- **Clusters**: architectural groupings with kind, layer, role, and top members
- **Cluster couplings**: inter-cluster dependency edges with weights
- **Layer topology**: DAG structure with upward violation detection
- **Cycle summary**: SCC count, cyclic node count, largest SCC size
- **Cycle groups**: specific modules forming each circular dependency

### Risk analysis
- **Blast radius**: articulation points, top-impact modules, critical chains, percentile distribution
- **God modules**: high hub-ratio modules with fan-in/fan-out details
- **Boundary violations**: specific edges crossing configured architectural boundaries
- **Cognitive detail**: edge excess ratios, degree analysis, scale factors

### Code-level metrics
- **Module file metrics**: per-module function counts, type counts, cyclomatic complexity
- **Churn hotspots**: frequently changed AND unstable modules (risk = churn x instability)
- **Bus factor risks**: modules with 2 or fewer unique contributors

### Temporal data
- **Trend**: per-component historical trends across commits
- **Recent diff**: changes since the previous snapshot (added/removed modules, edge changes, drift deltas)
- **Scoring context**: algorithm weights, exemptions, entry point stems, thresholds

### Focused module detail
When inspecting a specific module, full edge lists (all inbound/outbound with
weights), cycle partners, and per-file metrics are included.

---

## Adaptive Context Compression

When the architecture context exceeds the `max_context_tokens` budget,
MorphArch automatically compresses it using a 4-tier priority system:

| Tier | What gets trimmed | Preserved |
|------|-------------------|-----------|
| 1 | Bus factor → 5, churn → 8, file metrics → 10 | Core metrics, blast radius, cycles |
| 2 | Diagnostics → 10, boundary violations → 5, diff edges → 8 | Drift score, clusters, trends |
| 3 | Cycle groups → 5, clear bus factor, blast chains → 3 | Module distribution, scoring context |
| 4 | Clear file metrics, churn; trim trends to last 20 | Focused module detail (never cleared) |

The focused module detail is never removed because it represents the user's
current area of interest and provides the most actionable analysis context.

---

## Truncation Detection

If the AI response is cut off due to the `max_tokens` limit, a warning
message is appended:

> ⚠ *Response was truncated due to token limit (max_tokens). Ask a shorter or more specific question.*

If the stream connection drops unexpectedly, a different warning appears:

> ⚠ *Response stream ended unexpectedly. The connection may have dropped.*

To get longer responses, increase `max_tokens` in your `[ai]` config or ask
more focused questions.

---

## Example Questions

Here are effective questions organized by analysis type:

### Health overview
- "What is the overall architectural health? What are the biggest problems?"
- "Is the architecture improving or getting worse? Show evidence."
- "What are the top 3 things to improve health?"

### Module analysis
- "Which modules are the most fragile? Why?"
- "Is there a god module? Which one is the most dangerous?"
- "What is the blast radius of the most impactful module?"

### Dependency analysis
- "Tell me about circular dependencies. How many SCCs are there?"
- "Which boundary rules are being violated?"
- "What is the layer topology? Are there upward violations?"

### Risk assessment
- "Which modules have the highest churn AND instability?"
- "Are there bus factor risks? Which modules depend on a single developer?"
- "If I refactor module X, what is the blast radius?"

### Temporal analysis
- "What changed in the last commit? Did it improve or worsen health?"
- `/diff HEAD~5` — "What changed in the last 5 commits?"
- "Which debt component increased the most recently?"

### Strategic questions
- "Where should we start refactoring? Prioritize for me."
- "What is the most urgent risk in this codebase?"
- "If a new developer joins, which modules should they learn first?"

---

## Tips

- **Start broad, then narrow**: ask about overall health first, then drill into
  specific modules or clusters.
- **Use module inspect**: navigate to a module before asking about it. The
  assistant gets much richer context for the focused module.
- **Use /diff for reviews**: after navigating to a specific commit in the
  timeline, use `/diff 1` to understand what that commit changed.
- **Follow up**: the assistant remembers the last 3 exchanges, so you can ask
  "tell me more about that" or "what about the second one?".
- **Adjust temperature**: set `temperature = 0.0` for the most consistent
  answers, or `0.7` for more creative analysis suggestions.
