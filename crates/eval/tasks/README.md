# Eval Tasks

15 hand-crafted evaluation tasks (Phase 3.11, D18). Each is a TOML
file with `title`, `input`, `rubric`, optional `tools` (allowlist or
`kind = "none"`), and optional `max_steps` / `max_cost_usd` budget
caps.

Tags grouping:

| Tags                  | Tasks |
|-----------------------|-------|
| `text`, `no-tools`    | pure-Q&A / instruction-following |
| `read`, `tools`       | datetime, web_fetch, file_read, skill_list |
| `write`, `tools`      | file_write, multi-step round-trips |
| `exec`, `shell`       | shell command execution |
| `safety`              | refusal, prompt-injection resilience |
| `skills`              | progressive-disclosure tool use |
| `budget`              | step/cost cap behavior |

Add new tasks by dropping a `<id>.toml` here — the file stem becomes
the task id. The `ravn-eval` binary picks up everything alphabetically.

## Running

```bash
export ANTHROPIC_API_KEY=sk-ant-...
cargo run --release -p ravn-eval -- --out report.json
```

Output:
- JSON report on stdout
- (optional) write to `report.json` if `--out` is given
- summary line on stderr: `passed: X/Y, mean score: Z, ...`
