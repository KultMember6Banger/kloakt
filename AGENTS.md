# Kloakt

Cloaked headless browser for AI agents. Rust/V8, 30MB memory, stealth TLS fingerprinting.

## Build

```bash
cargo build --release
cargo build --release --features stealth  # anti-detection + tracker blocking
```

Requires Rust 1.75+. First build takes ~5 min (V8 compiles from source, cached after).

Binary output: `target/release/kloakt`

## Architecture

Rust workspace with 6 crates:
- `obscura-dom` — DOM tree implementation
- `obscura-net` — HTTP client, TLS, cookie jar
- `obscura-browser` — page lifecycle, navigation, JS evaluation
- `obscura-cdp` — Chrome DevTools Protocol server
- `obscura-js` — V8 bindings via deno_core
- `obscura-cli` — CLI binary (kloakt), extract/fetch/scrape/serve commands

The `extract` command runs a 3-phase pipeline: noise removal → text-density scoring → markdown conversion. SPA fallback pre-extracts meta/JSON-LD/noscript before DOM manipulation.

## Key Commands

```bash
kloakt extract <URL> --json --main --max-chars 3000
kloakt fetch <URL> --dump markdown --eval "document.title"
kloakt serve --port 9222 --stealth
kloakt scrape url1 url2 --concurrency 10 --format json
```

## Python API

```python
from kloakt import extract, fetch, scrape
```

## MCP Server

`mcp_server.py` exposes `kloakt_extract` and `kloakt_fetch` via stdio JSON-RPC.

## Code Style

- Rust: standard rustfmt, no clippy overrides
- JS evaluated in V8: ES5 compatible (no arrow functions, no const/let, no optional chaining in evaluated scripts). Use `var`, `function(){}`, `.indexOf()` instead of `.includes()`.
- Python wrappers: type hints, dataclasses, subprocess-based (no FFI).

## Testing

```bash
kloakt extract https://example.com --json          # basic HTML
kloakt extract https://hackerone.com/flickr --json  # SPA fallback
kloakt extract https://en.wikipedia.org/wiki/Web_scraping --main --max-chars 1000  # content capping
```

No automated test suite yet. Test manually against the three site types above.
