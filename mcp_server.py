"""Kloakt MCP server — exposes cloaked headless browser tools to AI agents.

Implements the Model Context Protocol (stdio JSON-RPC) so Claude Code
subagents can call extract/fetch/scrape as native tools.

Usage:
    python3 mcp_server.py

Add to .mcp.json or claude settings:
    "kloakt": {
        "command": "python3",
        "args": ["/home/kultmember6banger/ai-stack/obscura/mcp_server.py"]
    }
"""

import json
import sys
from kloakt import extract, fetch, scrape


TOOLS = [
    {
        "name": "kloakt_extract",
        "description": (
            "Extract clean markdown content from a web page using a headless browser "
            "with full JavaScript rendering. Strips nav/header/footer by default. "
            "Returns structured data: title, URL, markdown content, meta tags, timing. "
            "Use this instead of WebFetch when you need JS-rendered content or clean markdown."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to extract content from",
                },
                "main_only": {
                    "type": "boolean",
                    "description": "Strip nav/header/footer/sidebar (default: true)",
                    "default": True,
                },
                "format": {
                    "type": "string",
                    "enum": ["markdown", "text", "links"],
                    "description": "Output format (default: markdown)",
                    "default": "markdown",
                },
                "stealth": {
                    "type": "boolean",
                    "description": "Enable anti-detection mode",
                    "default": False,
                },
                "wait_until": {
                    "type": "string",
                    "enum": ["load", "domcontentloaded", "networkidle0"],
                    "description": "When to consider page loaded (default: load)",
                    "default": "load",
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector to wait for before extracting",
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Truncate content to N characters (0 = unlimited, default: 0)",
                    "default": 0,
                },
                "delay": {
                    "type": "integer",
                    "description": "Extra milliseconds to wait after load for async content (default: 0)",
                    "default": 0,
                },
            },
            "required": ["url"],
        },
    },
    {
        "name": "kloakt_fetch",
        "description": (
            "Low-level page fetch with JS rendering. Returns raw output as string. "
            "Supports HTML dump, text dump, link extraction, or arbitrary JS evaluation."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch",
                },
                "dump": {
                    "type": "string",
                    "enum": ["html", "text", "links", "markdown"],
                    "description": "Output format (default: text)",
                    "default": "text",
                },
                "eval_js": {
                    "type": "string",
                    "description": "JavaScript expression to evaluate on the page",
                },
                "stealth": {
                    "type": "boolean",
                    "description": "Enable anti-detection mode",
                    "default": False,
                },
            },
            "required": ["url"],
        },
    },
]


def handle_request(request: dict) -> dict:
    method = request.get("method", "")
    req_id = request.get("id")
    params = request.get("params", {})

    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": "kloakt",
                    "version": "0.1.0",
                },
            },
        }

    if method == "notifications/initialized":
        return None

    if method == "tools/list":
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {"tools": TOOLS},
        }

    if method == "tools/call":
        tool_name = params.get("name", "")
        args = params.get("arguments", {})

        try:
            if tool_name == "kloakt_extract":
                page = extract(
                    url=args["url"],
                    format=args.get("format", "markdown"),
                    main_only=args.get("main_only", True),
                    stealth=args.get("stealth", False),
                    wait_until=args.get("wait_until", "load"),
                    selector=args.get("selector"),
                    max_chars=args.get("max_chars", 0),
                    delay=args.get("delay", 0),
                )
                content = json.dumps({
                    "url": page.url,
                    "title": page.title,
                    "content": page.content,
                    "meta": page.meta,
                    "elapsed_ms": page.elapsed_ms,
                }, indent=2)
                return {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "content": [{"type": "text", "text": content}],
                    },
                }

            elif tool_name == "kloakt_fetch":
                result = fetch(
                    url=args["url"],
                    dump=args.get("dump", "text"),
                    eval_js=args.get("eval_js"),
                    stealth=args.get("stealth", False),
                )
                return {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "content": [{"type": "text", "text": result}],
                    },
                }

            else:
                return {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": {"code": -32601, "message": f"Unknown tool: {tool_name}"},
                }

        except Exception as e:
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [{"type": "text", "text": f"Error: {e}"}],
                    "isError": True,
                },
            }

    if method == "ping":
        return {"jsonrpc": "2.0", "id": req_id, "result": {}}

    return {
        "jsonrpc": "2.0",
        "id": req_id,
        "error": {"code": -32601, "message": f"Unknown method: {method}"},
    }


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue

        response = handle_request(request)
        if response is not None:
            sys.stdout.write(json.dumps(response) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
