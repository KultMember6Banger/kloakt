"""Kloakt — cloaked headless browser for AI agents.

Provides a clean API for AI agents to fetch, extract, and interact with web pages.
Requires the kloakt binary to be built and on PATH or at a known location.
"""

import json
import subprocess
import shutil
from pathlib import Path
from dataclasses import dataclass

BINARY_LOCATIONS = [
    Path.home() / ".cargo" / "bin" / "kloakt",
    Path(__file__).parent / "target" / "release" / "kloakt",
]


def _find_binary() -> str:
    found = shutil.which("kloakt")
    if found:
        return found
    for loc in BINARY_LOCATIONS:
        if loc.exists():
            return str(loc)
    raise FileNotFoundError(
        "kloakt binary not found. Build with: cd kloakt && cargo build --release --features stealth"
    )


@dataclass
class Page:
    url: str
    title: str
    content: str
    meta: dict
    elapsed_ms: int


def extract(
    url: str,
    format: str = "markdown",
    main_only: bool = True,
    stealth: bool = False,
    selector: str | None = None,
    wait_until: str = "load",
    max_chars: int = 0,
    delay: int = 0,
    timeout: int = 30,
) -> Page:
    """Extract content from a URL.

    Args:
        url: page URL
        format: markdown, text, or links
        main_only: strip nav/header/footer
        stealth: enable anti-detection
        selector: wait for CSS selector before extracting
        wait_until: load, domcontentloaded, or networkidle0
        max_chars: truncate content to N chars (0 = unlimited)
        delay: extra ms to wait after load for async content
        timeout: seconds before giving up
    """
    cmd = [_find_binary(), "extract", url, "--format", format, "--json",
           "--wait-until", wait_until]
    if main_only:
        cmd.append("--main")
    if stealth:
        cmd.append("--stealth")
    if selector:
        cmd.extend(["--selector", selector])
    if max_chars > 0:
        cmd.extend(["--max-chars", str(max_chars)])
    if delay > 0:
        cmd.extend(["--delay", str(delay)])

    result = subprocess.run(
        cmd, capture_output=True, text=True, timeout=timeout
    )

    if result.returncode != 0:
        raise RuntimeError(f"obscura failed: {result.stderr.strip()}")

    data = json.loads(result.stdout)
    return Page(
        url=data["url"],
        title=data["title"],
        content=data["content"],
        meta=data.get("meta", {}),
        elapsed_ms=data.get("elapsed_ms", 0),
    )


def fetch(
    url: str,
    dump: str = "text",
    eval_js: str | None = None,
    stealth: bool = False,
    timeout: int = 30,
) -> str:
    """Low-level fetch — returns raw output as string."""
    cmd = [_find_binary(), "fetch", url, "--dump", dump, "--quiet"]
    if eval_js:
        cmd.extend(["--eval", eval_js])
    if stealth:
        cmd.append("--stealth")

    result = subprocess.run(
        cmd, capture_output=True, text=True, timeout=timeout
    )

    if result.returncode != 0:
        raise RuntimeError(f"obscura failed: {result.stderr.strip()}")

    return result.stdout.strip()


def scrape(
    urls: list[str],
    eval_js: str | None = None,
    concurrency: int = 10,
    timeout: int = 60,
) -> list[dict]:
    """Parallel scrape multiple URLs."""
    cmd = [_find_binary(), "scrape"] + urls + [
        "--concurrency", str(concurrency),
        "--format", "json",
        "--timeout", str(timeout),
    ]
    if eval_js:
        cmd.extend(["--eval", eval_js])

    result = subprocess.run(
        cmd, capture_output=True, text=True, timeout=timeout + 30
    )

    if result.returncode != 0:
        raise RuntimeError(f"obscura failed: {result.stderr.strip()}")

    data = json.loads(result.stdout)
    return data.get("results", [])


if __name__ == "__main__":
    import sys
    url = sys.argv[1] if len(sys.argv) > 1 else "https://example.com"
    page = extract(url)
    print(f"Title: {page.title}")
    print(f"URL: {page.url}")
    print(f"Time: {page.elapsed_ms}ms")
    print(f"Content: {len(page.content)} chars")
    print()
    print(page.content[:500])
