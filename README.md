<h2 align="center">Kloakt</h2>

<p align="center">
  <strong>Cloaked headless browser for AI agents.</strong><br>
  Lightweight, stealthy, built in Rust. Based on <a href="https://github.com/h4ckf0r0day/obscura">Obscura</a>.
</p>

---

Kloakt is a headless browser built for AI agents. It runs JavaScript via V8, extracts clean markdown from any page (including SPAs), and exposes tools via MCP for Claude Code and other AI systems.

### Why Kloakt?

| Metric       | Kloakt       | Headless Chrome |
|--------------|--------------|------------------|
| Memory       | **30 MB**    | 200+ MB          |
| Binary size  | **70 MB**    | 300+ MB          |
| Anti-detect  | **Built-in** | None             |
| Page load    | **85 ms**    | ~500 ms          |
| Startup      | **Instant**  | ~2s              |
| SPA extract  | **Yes**      | Manual           |

## Install

### Build from source

```bash
git clone https://github.com/KultMember6Banger/kloakt.git
cd kloakt
cargo build --release

# With stealth mode (anti-detection + tracker blocking)
cargo build --release --features stealth
```

Requires Rust 1.75+ ([rustup.rs](https://rustup.rs)). First build takes ~5 min (V8 compiles from source, cached after).

## Quick Start

### Extract content (AI agent use)

```bash
# Clean markdown from any page
kloakt extract https://example.com --main

# Structured JSON with metadata
kloakt extract https://example.com --main --json

# Cap output for agent context windows
kloakt extract https://en.wikipedia.org/wiki/Rust --main --json --max-chars 3000

# Wait for SPA hydration
kloakt extract https://example.com --delay 2000 --json
```

### Fetch a page

```bash
# Get the page title
kloakt fetch https://example.com --eval "document.title"

# Extract all links
kloakt fetch https://example.com --dump links

# Render JavaScript and dump markdown
kloakt fetch https://news.ycombinator.com --dump markdown

# Wait for dynamic content
kloakt fetch https://example.com --wait-until networkidle0
```

### Start the CDP server

```bash
kloakt serve --port 9222

# With stealth mode
kloakt serve --port 9222 --stealth
```

### Scrape in parallel

```bash
kloakt scrape url1 url2 url3 ... \
  --concurrency 25 \
  --eval "document.querySelector('h1').textContent" \
  --format json
```

## Smart Extraction

The `extract` command uses a multi-phase pipeline optimized for AI agents:

1. **Noise removal** — strips cookie banners, ads, popups, nav, social widgets
2. **Content scoring** — text-density algorithm (Readability-like) finds the main content block
3. **Markdown conversion** — DOM-to-markdown with absolute URL resolution
4. **SPA fallback** — when JS rendering fails, extracts from meta tags, Open Graph, JSON-LD, and noscript content

Works on static HTML, server-rendered pages, and pure client-side SPAs (React, Vue, etc.).

## Python API

```python
from kloakt import extract, fetch, scrape

# Extract clean markdown
page = extract("https://example.com")
print(page.title, page.content, page.meta)

# Cap output length
page = extract("https://example.com", max_chars=3000)

# Wait for SPA content
page = extract("https://example.com", delay=2000)

# Raw fetch
html = fetch("https://example.com", dump="html")
title = fetch("https://example.com", eval_js="document.title")

# Parallel scrape
results = scrape(["https://a.com", "https://b.com"], concurrency=5)
```

## MCP Server (Claude Code)

Kloakt includes an MCP server for use as a Claude Code tool:

```json
{
  "mcpServers": {
    "kloakt": {
      "command": "python3",
      "args": ["/path/to/kloakt/mcp_server.py"]
    }
  }
}
```

Exposes `kloakt_extract` and `kloakt_fetch` as native tools.

## Puppeteer / Playwright

### Puppeteer

```javascript
import puppeteer from 'puppeteer-core';

const browser = await puppeteer.connect({
  browserWSEndpoint: 'ws://127.0.0.1:9222/devtools/browser',
});

const page = await browser.newPage();
await page.goto('https://news.ycombinator.com');
const stories = await page.evaluate(() =>
  Array.from(document.querySelectorAll('.titleline > a'))
    .map(a => ({ title: a.textContent, url: a.href }))
);
await browser.disconnect();
```

### Playwright

```javascript
import { chromium } from 'playwright-core';

const browser = await chromium.connectOverCDP({
  endpointURL: 'ws://127.0.0.1:9222',
});

const page = await browser.newContext().then(ctx => ctx.newPage());
await page.goto('https://en.wikipedia.org/wiki/Web_scraping');
console.log(await page.title());
await browser.close();
```

## Stealth Mode

Enable with `--features stealth`.

- Per-session fingerprint randomization (GPU, screen, canvas, audio, battery)
- Realistic `navigator.userAgentData` (Chrome 145, high-entropy values)
- `event.isTrusted = true` for dispatched events
- Native function masking (`Function.prototype.toString()` → `[native code]`)
- `navigator.webdriver = undefined`
- 3,520 tracker domains blocked

## CLI Reference

### `kloakt extract <URL>`

| Flag | Default | Description |
|------|---------|-------------|
| `--format` | `markdown` | Output: `markdown`, `text`, or `links` |
| `--main` | off | Strip nav, header, footer, sidebar |
| `--json` | off | Structured JSON: title, URL, content, meta |
| `--max-chars` | unlimited | Truncate content to N characters |
| `--delay` | `0` | Extra ms to wait after load |
| `--stealth` | off | Anti-detection mode |
| `--selector` | — | Wait for CSS selector |
| `--wait-until` | `load` | `load`, `domcontentloaded`, `networkidle0` |

### `kloakt fetch <URL>`

| Flag | Default | Description |
|------|---------|-------------|
| `--dump` | `html` | Output: `html`, `text`, `links`, `markdown` |
| `--eval` | — | JavaScript expression to evaluate |
| `--wait-until` | `load` | Wait condition |
| `--selector` | — | Wait for CSS selector |
| `--stealth` | off | Anti-detection mode |
| `--quiet` | off | Suppress banner |

### `kloakt serve`

| Flag | Default | Description |
|------|---------|-------------|
| `--port` | `9222` | WebSocket port |
| `--proxy` | — | HTTP/SOCKS5 proxy URL |
| `--stealth` | off | Anti-detection + tracker blocking |
| `--workers` | `1` | Parallel workers |

### `kloakt scrape <URL...>`

| Flag | Default | Description |
|------|---------|-------------|
| `--concurrency` | `10` | Parallel workers |
| `--eval` | — | JS expression per page |
| `--format` | `json` | Output: `json` or `text` |

## CDP API

Full Chrome DevTools Protocol support for Puppeteer/Playwright compatibility.

| Domain | Methods |
|--------|---------|
| **Target** | createTarget, closeTarget, attachToTarget, createBrowserContext, disposeBrowserContext |
| **Page** | navigate, getFrameTree, addScriptToEvaluateOnNewDocument, lifecycleEvents |
| **Runtime** | evaluate, callFunctionOn, getProperties, addBinding |
| **DOM** | getDocument, querySelector, querySelectorAll, getOuterHTML, resolveNode |
| **Network** | enable, setCookies, getCookies, setExtraHTTPHeaders, setUserAgentOverride |
| **Fetch** | enable, continueRequest, fulfillRequest, failRequest |
| **Storage** | getCookies, setCookies, deleteCookies |
| **Input** | dispatchMouseEvent, dispatchKeyEvent |

## License

Apache 2.0 — Based on [Obscura](https://github.com/h4ckf0r0day/obscura) by h4ckf0r0day.
