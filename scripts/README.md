# scripts/

One-off developer utilities. None of these run in CI or at build time.

## `distill_axioms.py`

Generates `assets/axioms.json` from the text of *The Rust Programming
Language* book. The baked-in JSON is what the `axioms` MCP tool queries
at runtime; this script is only needed when you want to regenerate or
extend that corpus.

**Prerequisites**

1. Python 3.9+ with `openai` installed: `pip install openai`
2. `OPENAI_API_KEY` in your environment.
3. The Rust Book source checked out into `assets/rust-book/`:

   ```bash
   git clone https://github.com/rust-lang/book.git assets/rust-book
   ```

   The Rust Book repo is **not** vendored in this tree — it lives under
   `assets/rust-book/` at clone time but is intentionally excluded from
   version control (it has its own `.git/`, ~200MB with history).

**Running**

```bash
python scripts/distill_axioms.py
```

Iterates every `.md` under `assets/rust-book/src/`, chunks it by `##`
heading, and asks GPT-4o to extract axiom-shaped rules per chunk. Output
is written to `assets/axioms.json` — commit the result if you are
refreshing the shipped corpus.

The script is idempotent but expensive (OpenAI API calls for every
chunk); there is no incremental mode yet.
