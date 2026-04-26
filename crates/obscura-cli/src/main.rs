use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, Subcommand};
use obscura_browser::{BrowserContext, Page};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::time::{timeout, Duration};

#[derive(Parser)]
#[command(name = "kloakt", about = "Kloakt - Cloaked headless browser for AI agents")]
struct Args {
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Command>,

    #[arg(short, long, default_value_t = 9222)]
    port: u16,

    #[arg(long)]
    proxy: Option<String>,

    #[arg(long)]
    obey_robots: bool,

    #[arg(long)]
    user_agent: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(short, long, default_value_t = 9222)]
        port: u16,

        #[arg(long)]
        proxy: Option<String>,

        #[arg(long)]
        user_agent: Option<String>,

        #[arg(long)]
        stealth: bool,

        #[arg(long, default_value_t = 1)]
        workers: u16,
    },

    Fetch {
        url: String,

        #[arg(long, default_value = "html")]
        dump: DumpFormat,

        #[arg(long)]
        selector: Option<String>,

        #[arg(long, default_value_t = 5)]
        wait: u64,

        #[arg(long, default_value = "load")]
        wait_until: String,

        #[arg(long)]
        user_agent: Option<String>,

        #[arg(long)]
        stealth: bool,

        #[arg(long, short)]
        eval: Option<String>,

        #[arg(long, short)]
        quiet: bool,
    },

    Scrape {
        urls: Vec<String>,

        #[arg(long, short)]
        eval: Option<String>,

        #[arg(long, default_value_t = 10)]
        concurrency: usize,

        #[arg(long, default_value = "json")]
        format: String,

        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..))]
        timeout: u64,
    },

    Extract {
        url: String,

        #[arg(long, default_value = "markdown")]
        format: ExtractFormat,

        #[arg(long)]
        selector: Option<String>,

        #[arg(long, default_value_t = 10)]
        wait: u64,

        #[arg(long, default_value = "load")]
        wait_until: String,

        #[arg(long)]
        stealth: bool,

        #[arg(long)]
        json: bool,

        #[arg(long)]
        main: bool,

        #[arg(long, help = "Truncate content to N characters (0 = unlimited)")]
        max_chars: Option<usize>,

        #[arg(long, default_value_t = 0, help = "Extra milliseconds to wait after page load for async content")]
        delay: u64,
    },

}


#[derive(Clone, Debug, clap::ValueEnum)]
enum DumpFormat {
    Html,
    Text,
    Links,
    Markdown,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum ExtractFormat {
    Markdown,
    Text,
    Links,
}

fn print_banner(port: u16) {
    println!(r#"
  _  ___             _    _
 | |/ / |           | |  | |
 | ' /| | ___   __ _| | _| |_
 |  < | |/ _ \ / _` | |/ / __|
 | . \| | (_) | (_| |   <| |_
 |_|\_\_|\___/ \__,_|_|\_\\__|

  Cloaked Headless Browser v0.2.0
  CDP server: ws://127.0.0.1:{}/devtools/browser
"#, port);
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let filter = if args.verbose { "debug" } else { "warn" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_writer(std::io::stderr)
        .init();

    match args.command {
        Some(Command::Serve { port, proxy, user_agent, stealth, workers }) => {
            print_banner(port);
            if let Some(ref proxy) = proxy {
                tracing::info!("Using proxy: {}", proxy);
            }
            if let Some(ref ua) = user_agent {
                tracing::info!("User-Agent: {}", ua);
            }
            if stealth {
                #[cfg(feature = "stealth")]
                tracing::info!(
                    "Stealth mode enabled (TLS fingerprint impersonation + tracker blocking)"
                );
                #[cfg(not(feature = "stealth"))]
                tracing::info!("Stealth mode enabled (tracker blocking)");
            }

            if workers > 1 {
                tracing::info!("{} worker processes", workers);
                run_multi_worker_serve(port, workers, proxy, stealth).await?;
            } else {
                obscura_cdp::start_with_options(port, proxy, stealth).await?;
            }
        }
        Some(Command::Fetch { url, dump, selector, wait, wait_until, user_agent, stealth, eval, quiet }) => {
            run_fetch(&url, dump, selector, wait, &wait_until, user_agent, stealth, eval, quiet).await?;
        }
        Some(Command::Scrape { urls, eval, concurrency, format, timeout }) => {
            run_parallel_scrape(urls, eval, concurrency, &format, timeout).await?;
        }
        Some(Command::Extract { url, format, selector, wait, wait_until, stealth, json, main, max_chars, delay }) => {
            run_extract(&url, format, selector, wait, &wait_until, stealth, json, main, max_chars, delay).await?;
        }
        None => {
            print_banner(args.port);
            if let Some(ref proxy) = args.proxy {
                tracing::info!("Using proxy: {}", proxy);
            }
            obscura_cdp::start_with_options(args.port, args.proxy, false).await?;
        }
    }

    Ok(())
}

async fn run_multi_worker_serve(
    port: u16,
    workers: u16,
    proxy: Option<String>,
    stealth: bool,
) -> anyhow::Result<()> {
    use tokio::net::TcpListener;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let exe = std::env::current_exe()?;
    let mut children = Vec::new();

    for i in 0..workers {
        let worker_port = port + 1 + i;
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("serve").arg("--port").arg(worker_port.to_string());
        if let Some(ref p) = proxy {
            cmd.arg("--proxy").arg(p);
        }
        if stealth {
            cmd.arg("--stealth");
        }
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        let child = cmd.spawn()?;
        tracing::info!("Worker {} on port {}", i + 1, worker_port);
        children.push(child);
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("Load balancer on port {}, {} workers", port, workers);

    let mut next_worker: u16 = 0;

    loop {
        let (client_stream, peer_addr) = listener.accept().await?;
        let worker_port = port + 1 + (next_worker % workers);
        next_worker = next_worker.wrapping_add(1);

        tracing::debug!("Routing {} to worker port {}", peer_addr, worker_port);

        let mut peek_buf = [0u8; 4];
        client_stream.peek(&mut peek_buf).await?;

        if &peek_buf == b"GET " {
            let mut full_peek = [0u8; 256];
            let n = client_stream.peek(&mut full_peek).await?;
            let request_line = String::from_utf8_lossy(&full_peek[..n]);

            if request_line.contains("/json") {
                let worker_addr = format!("127.0.0.1:{}", worker_port);
                match tokio::net::TcpStream::connect(&worker_addr).await {
                    Ok(mut worker_stream) => {
                        tokio::spawn(async move {
                            let std_stream = match client_stream.into_std() {
                                Ok(s) => s,
                                Err(e) => {
                                    tracing::error!(
                                        "/json: failed to convert client to std stream: {}",
                                        e
                                    );
                                    return;
                                }
                            };
                            let mut client = match tokio::net::TcpStream::from_std(std_stream) {
                                Ok(c) => c,
                                Err(e) => {
                                    tracing::error!(
                                        "/json: failed to recreate tokio TcpStream: {}",
                                        e
                                    );
                                    return;
                                }
                            };
                            let _ = tokio::io::copy_bidirectional(
                                &mut client,
                                &mut worker_stream,
                            )
                            .await;
                        });
                    }
                    Err(e) => {
                        tracing::warn!("/json worker {} unreachable: {}", worker_addr, e);
                        tokio::spawn(async move {
                            let mut s = client_stream;
                            let _ = s
                                .write_all(
                                    b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n",
                                )
                                .await;
                            let _ = s.shutdown().await;
                        });
                    }
                }
                continue;
            }
        }

        let worker_addr = format!("127.0.0.1:{}", worker_port);
        tokio::spawn(async move {
            match tokio::net::TcpStream::connect(&worker_addr).await {
                Ok(mut worker_stream) => {
                    let mut client = client_stream;
                    let _ =
                        tokio::io::copy_bidirectional(&mut client, &mut worker_stream).await;
                }
                Err(e) => {
                    tracing::warn!("worker {} unreachable: {}", worker_addr, e);
                    let mut s = client_stream;
                    let _ = s
                        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                        .await;
                    let _ = s.shutdown().await;
                }
            }
        });
    }
}

async fn run_fetch(
    url_str: &str,
    dump: DumpFormat,
    selector: Option<String>,
    wait_secs: u64,
    wait_until: &str,
    user_agent: Option<String>,
    stealth: bool,
    eval: Option<String>,
    quiet: bool,
) -> anyhow::Result<()> {
    let context = Arc::new(BrowserContext::with_options("fetch".to_string(), None, stealth));
    let mut page = Page::new("fetch-page".to_string(), context);

    if let Some(ref ua) = user_agent {
        page.http_client.set_user_agent(ua).await;
    }

    let wait_condition = obscura_browser::lifecycle::WaitUntil::from_str(wait_until);

    if !quiet {
        eprintln!("Fetching {}...", url_str);
    }

    page.navigate_with_wait(url_str, wait_condition).await.map_err(|e| {
        anyhow::anyhow!("Failed to navigate to {}: {}", url_str, e)
    })?;

    if !quiet {
        eprintln!("Page loaded: {} - \"{}\"", page.url_string(), page.title);
    }

    if let Some(ref sel) = selector {
        let found = wait_for_selector(&mut page, sel, wait_secs).await;
        if !found {
            eprintln!("Warning: selector '{}' not found after {}s", sel, wait_secs);
        }
    }

    if let Some(ref expr) = eval {
        let result = page.evaluate(expr);
        match result {
            serde_json::Value::String(s) => println!("{}", s),
            serde_json::Value::Null => println!("null"),
            other => println!("{}", other),
        }
        return Ok(());
    }

    match dump {
        DumpFormat::Html => {
            dump_html(&page);
        }
        DumpFormat::Text => {
            dump_text(&mut page);
        }
        DumpFormat::Links => {
            dump_links(&page);
        }
        DumpFormat::Markdown => {
            dump_markdown(&mut page);
        }
    }

    Ok(())
}

async fn wait_for_selector(page: &mut Page, selector: &str, timeout_secs: u64) -> bool {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);
    loop {
        let found = page.with_dom(|dom| {
            dom.query_selector(selector).ok().flatten().is_some()
        }).unwrap_or(false);

        if found {
            return true;
        }

        if tokio::time::Instant::now() >= deadline {
            return false;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
}

fn dump_html(page: &Page) {
    page.with_dom(|dom| {
        if let Ok(Some(html_node)) = dom.query_selector("html") {
            let html = dom.outer_html(html_node);
            println!("<!DOCTYPE html>");
            println!("{}", html);
        } else {
            let doc = dom.document();
            let html = dom.inner_html(doc);
            println!("{}", html);
        }
    });
}

fn dump_text(page: &mut Page) {
    page.with_dom(|dom| {
        if let Ok(Some(body)) = dom.query_selector("body") {
            let text = extract_readable_text(dom, body);
            println!("{}", text.trim());
        }
    });
}

fn extract_readable_text(dom: &obscura_dom::DomTree, node_id: obscura_dom::NodeId) -> String {
    use obscura_dom::NodeData;

    let mut result = String::new();
    let node = match dom.get_node(node_id) {
        Some(n) => n,
        None => return result,
    };

    match &node.data {
        NodeData::Text { contents } => {
            let trimmed = contents.trim();
            if !trimmed.is_empty() {
                result.push_str(trimmed);
            }
        }
        NodeData::Element { name, .. } => {
            let tag = name.local.as_ref();
            let is_block = matches!(
                tag,
                "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                    | "li" | "tr" | "br" | "hr" | "blockquote" | "pre"
                    | "section" | "article" | "header" | "footer" | "nav"
                    | "main" | "aside" | "figure" | "figcaption" | "table"
                    | "thead" | "tbody" | "tfoot" | "dl" | "dt" | "dd"
                    | "ul" | "ol"
            );

            if tag == "script" || tag == "style" {
                return result;
            }

            if is_block {
                result.push('\n');
            }

            for child_id in dom.children(node_id) {
                result.push_str(&extract_readable_text(dom, child_id));
            }

            if is_block {
                result.push('\n');
            }
        }
        _ => {
            for child_id in dom.children(node_id) {
                result.push_str(&extract_readable_text(dom, child_id));
            }
        }
    }

    result
}

async fn run_parallel_scrape(
    urls: Vec<String>,
    eval: Option<String>,
    concurrency: usize,
    format: &str,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    let total = urls.len();
    let start = Instant::now();

    eprintln!(
        "Scraping {} URLs with {} concurrent workers (per-worker timeout: {}s)...",
        total, concurrency, timeout_secs
    );

    let worker_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("obscura-worker")))
        .unwrap_or_else(|| std::path::PathBuf::from("obscura-worker"));

    if !worker_path.exists() {
        anyhow::bail!(
            "Worker binary not found at {}. Build with: cargo build --release",
            worker_path.display()
        );
    }

    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let eval = Arc::new(eval);
    let worker_path = Arc::new(worker_path);
    let worker_timeout = Duration::from_secs(timeout_secs);
    let read_timeout = Duration::from_secs(timeout_secs.min(30));
    let shutdown_timeout = Duration::from_secs(5);

    let mut handles = Vec::new();

    for (i, url) in urls.into_iter().enumerate() {
        let sem = semaphore.clone();
        let eval = eval.clone();
        let worker_path = worker_path.clone();

        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let task_start = Instant::now();

            let mut child = match TokioCommand::new(worker_path.as_ref())
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    return serde_json::json!({
                        "url": url,
                        "error": format!("Failed to spawn worker: {}", e),
                        "time_ms": task_start.elapsed().as_millis(),
                    });
                }
            };

            let mut stdin = match child.stdin.take() {
                Some(stdin) => stdin,
                None => {
                    let _ = timeout(shutdown_timeout, child.kill()).await;
                    return serde_json::json!({
                        "url": url,
                        "error": "Failed to open worker stdin",
                        "time_ms": task_start.elapsed().as_millis(),
                    });
                }
            };
            let stdout = match child.stdout.take() {
                Some(stdout) => stdout,
                None => {
                    let _ = timeout(shutdown_timeout, child.kill()).await;
                    return serde_json::json!({
                        "url": url,
                        "error": "Failed to open worker stdout",
                        "time_ms": task_start.elapsed().as_millis(),
                    });
                }
            };
            let mut reader = BufReader::new(stdout);

            let worker_result: Result<serde_json::Value, String> = match timeout(worker_timeout, async {
                let nav_cmd = serde_json::json!({"cmd": "navigate", "url": url});
                let mut line = serde_json::to_string(&nav_cmd).unwrap();
                line.push('\n');
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    return Err("Write failed".to_string());
                }
                if stdin.flush().await.is_err() {
                    return Err("Write failed".to_string());
                }

                let mut resp_line = String::new();
                match timeout(read_timeout, reader.read_line(&mut resp_line)).await {
                    Ok(Ok(bytes)) if bytes > 0 => {}
                    Ok(Ok(_)) | Ok(Err(_)) => return Err("Read failed".to_string()),
                    Err(_) => return Err("timeout".to_string()),
                };

                let nav_resp: serde_json::Value =
                    serde_json::from_str(resp_line.trim()).unwrap_or(serde_json::json!({"ok": false}));

                if !nav_resp["ok"].as_bool().unwrap_or(false) {
                    return Err(
                        nav_resp["error"]
                            .as_str()
                            .unwrap_or("navigate failed")
                            .to_string(),
                    );
                }

                let title = nav_resp["result"]["title"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();

                let eval_result = if let Some(ref expr) = *eval {
                    let eval_cmd = serde_json::json!({"cmd": "evaluate", "expression": expr});
                    let mut line = serde_json::to_string(&eval_cmd).unwrap();
                    line.push('\n');
                    if stdin.write_all(line.as_bytes()).await.is_err() {
                        return Err("Write failed".to_string());
                    }
                    if stdin.flush().await.is_err() {
                        return Err("Write failed".to_string());
                    }

                    let mut resp_line = String::new();
                    match timeout(read_timeout, reader.read_line(&mut resp_line)).await {
                        Ok(Ok(bytes)) if bytes > 0 => {
                            let resp: serde_json::Value = serde_json::from_str(resp_line.trim())
                                .unwrap_or(serde_json::json!({"ok": false}));
                            resp["result"].clone()
                        }
                        Ok(Ok(_)) | Ok(Err(_)) => return Err("Read failed".to_string()),
                        Err(_) => return Err("timeout".to_string()),
                    }
                } else {
                    serde_json::Value::Null
                };

                let shutdown_cmd = serde_json::json!({"cmd": "shutdown"});
                let mut line = serde_json::to_string(&shutdown_cmd).unwrap();
                line.push('\n');
                let _ = stdin.write_all(line.as_bytes()).await;
                let _ = stdin.flush().await;
                let _ = timeout(shutdown_timeout, child.wait()).await;

                Ok(serde_json::json!({
                    "url": url,
                    "title": title,
                    "eval": eval_result,
                    "time_ms": task_start.elapsed().as_millis(),
                    "worker": i,
                }))
            })
            .await
            {
                Ok(result) => result,
                Err(_) => Err("timeout".to_string()),
            };

            match worker_result {
                Ok(result) => result,
                Err(error) => {
                    let _ = timeout(shutdown_timeout, child.kill()).await;
                    serde_json::json!({
                        "url": url,
                        "error": error,
                        "time_ms": task_start.elapsed().as_millis(),
                    })
                }
            }
        });

        handles.push(handle);
    }

    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => results.push(serde_json::json!({"error": e.to_string()})),
        }
    }

    let total_time = start.elapsed();

    if format == "json" {
        let output = serde_json::json!({
            "total_urls": total,
            "concurrency": concurrency,
            "total_time_ms": total_time.as_millis(),
            "avg_time_ms": total_time.as_millis() as f64 / total as f64,
            "results": results,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        for r in &results {
            let url = r["url"].as_str().unwrap_or("?");
            let title = r["title"].as_str().unwrap_or("");
            let time = r["time_ms"].as_u64().unwrap_or(0);
            let eval = &r["eval"];
            if eval.is_null() {
                println!("{}ms\t{}\t{}", time, url, title);
            } else {
                println!("{}ms\t{}\t{}", time, url, eval);
            }
        }
        eprintln!(
            "\nTotal: {}ms for {} URLs ({} concurrent)",
            total_time.as_millis(),
            total,
            concurrency
        );
    }

    Ok(())
}

async fn run_extract(
    url_str: &str,
    format: ExtractFormat,
    selector: Option<String>,
    wait_secs: u64,
    wait_until: &str,
    stealth: bool,
    json_output: bool,
    main_only: bool,
    max_chars: Option<usize>,
    delay_ms: u64,
) -> anyhow::Result<()> {
    let context = Arc::new(BrowserContext::with_options("extract".to_string(), None, stealth));
    let mut page = Page::new("extract-page".to_string(), context);

    let wait_condition = obscura_browser::lifecycle::WaitUntil::from_str(wait_until);
    let start = Instant::now();

    page.navigate_with_wait(url_str, wait_condition).await.map_err(|e| {
        anyhow::anyhow!("Failed to navigate to {}: {}", url_str, e)
    })?;

    if let Some(ref sel) = selector {
        let found = wait_for_selector(&mut page, sel, wait_secs).await;
        if !found {
            eprintln!("Warning: selector '{}' not found after {}s", sel, wait_secs);
        }
    }

    if delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }

    if main_only {
        page.evaluate(r#"
            (function() {
                var selectors = ['nav', 'header', 'footer', '[role="navigation"]',
                    '[role="banner"]', '[role="contentinfo"]', '.sidebar', '#sidebar',
                    '.nav', '.menu', '.header', '.footer', '.breadcrumb', '.pagination',
                    '.cookie-banner', '.ad', '.advertisement'];
                selectors.forEach(function(sel) {
                    var els = document.querySelectorAll(sel);
                    for (var i = 0; i < els.length; i++) els[i].remove();
                });
            })()
        "#);
    }

    let title = page.title.clone();
    let url = page.url_string();
    let elapsed_ms = start.elapsed().as_millis();

    // Pre-extract fallback data BEFORE smart_extract — it removes meta/script/noscript from DOM
    let parsed_meta: serde_json::Value = match page.evaluate(
        r#"JSON.stringify(Array.from(document.querySelectorAll('meta')).reduce(function(a,m){var n=(m.getAttribute('name')||'').toLowerCase();var p=(m.getAttribute('property')||'').toLowerCase();var v=m.getAttribute('content')||'';if(!v)return a;if(n==='description'||p==='description')a.desc=v;if(p==='og:title')a.ogTitle=v;if(p==='og:description')a.ogDesc=v;if(p==='og:image')a.ogImage=v;if(p==='og:url')a.ogUrl=v;return a},{}))"#
    ) {
        serde_json::Value::String(s) => serde_json::from_str(&s).unwrap_or(serde_json::json!({})),
        _ => serde_json::json!({}),
    };
    let pre_noscript = page.evaluate(
        r#"Array.from(document.querySelectorAll('noscript')).map(function(n){return n.textContent||''}).filter(function(t){return t.length>50&&t.toLowerCase().indexOf('javascript')===-1&&t.toLowerCase().indexOf('enable')===-1}).join('\n\n')"#
    );
    let pre_jsonld = page.evaluate(
        r#"Array.from(document.querySelectorAll('script[type="application/ld+json"]')).map(function(el){try{var d=JSON.parse(el.textContent);var p=[];if(d.name)p.push('**'+d.name+'**');if(d.description)p.push(d.description);if(d.articleBody)p.push(d.articleBody);return p.join('\n\n')}catch(e){return ''}}).filter(function(s){return s}).join('\n\n')"#
    );
    let pre_canonical = match page.evaluate(
        r#"(document.querySelector('link[rel="canonical"]')||{}).href||''"#
    ) {
        serde_json::Value::String(s) => s,
        _ => String::new(),
    };

    let mut content = match format {
        ExtractFormat::Markdown => {
            smart_extract(&mut page)
        }
        ExtractFormat::Text => {
            get_text(&mut page)
        }
        ExtractFormat::Links => {
            get_links_text(&page)
        }
    };

    // Fallback: assemble from pre-extracted meta/structured data
    if content.trim().is_empty() || content.trim().len() < 50 {
        let mut parts: Vec<String> = Vec::new();

        if !title.trim().is_empty() {
            parts.push(format!("# {}", title.trim()));
        }

        let desc = parsed_meta.get("desc").and_then(|v| v.as_str()).unwrap_or("");
        let og_title = parsed_meta.get("ogTitle").and_then(|v| v.as_str()).unwrap_or("");
        let og_desc = parsed_meta.get("ogDesc").and_then(|v| v.as_str()).unwrap_or("");
        let og_url = parsed_meta.get("ogUrl").and_then(|v| v.as_str()).unwrap_or("");
        let og_image = parsed_meta.get("ogImage").and_then(|v| v.as_str()).unwrap_or("");

        if !og_title.is_empty() {
            parts.push(format!("**{}**", og_title.trim()));
        }
        if !desc.is_empty() {
            parts.push(desc.trim().to_string());
        }
        if !og_desc.is_empty() && og_desc != desc {
            parts.push(og_desc.trim().to_string());
        }
        if !og_url.is_empty() {
            parts.push(format!("URL: {}", og_url));
        }
        if !og_image.is_empty() {
            parts.push(format!("![og:image]({})", og_image));
        }

        if let serde_json::Value::String(s) = pre_noscript {
            if !s.trim().is_empty() {
                parts.push(s.trim().to_string());
            }
        }

        if let serde_json::Value::String(s) = pre_jsonld {
            if !s.trim().is_empty() {
                parts.push(s.trim().to_string());
            }
        }

        let fallback_content = parts.join("\n\n");
        if !fallback_content.trim().is_empty() {
            content = fallback_content;
        }
    }

    if let Some(cap) = max_chars {
        if cap > 0 && content.len() > cap {
            let truncated = &content[..content.floor_char_boundary(cap)];
            let last_break = truncated.rfind('\n').unwrap_or(cap);
            content = format!("{}\n\n[...truncated at {} chars]", &content[..last_break], last_break);
        }
    }

    if json_output {
        let lang = match page.evaluate("document.documentElement.lang||''") {
            serde_json::Value::String(s) => s,
            _ => String::new(),
        };
        let meta_obj = serde_json::json!({
            "description": parsed_meta.get("desc").and_then(|v| v.as_str()).unwrap_or(""),
            "og_title": parsed_meta.get("ogTitle").and_then(|v| v.as_str()).unwrap_or(""),
            "og_description": parsed_meta.get("ogDesc").and_then(|v| v.as_str()).unwrap_or(""),
            "og_image": parsed_meta.get("ogImage").and_then(|v| v.as_str()).unwrap_or(""),
            "canonical": pre_canonical,
            "lang": lang,
        });

        let output = serde_json::json!({
            "url": url,
            "title": title,
            "content": content,
            "meta": meta_obj,
            "elapsed_ms": elapsed_ms,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", content);
    }

    Ok(())
}

fn get_markdown(page: &mut Page) -> String {
    let code = MARKDOWN_JS;
    let result = page.evaluate(code);
    match result {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    }
}

fn get_text(page: &mut Page) -> String {
    let mut out = String::new();
    page.with_dom(|dom| {
        if let Ok(Some(body)) = dom.query_selector("body") {
            out = extract_readable_text(dom, body).trim().to_string();
        }
    });
    out
}

fn get_links_text(page: &Page) -> String {
    let base_url = page.url.clone();
    let mut lines = Vec::new();
    page.with_dom(|dom| {
        let links = dom.query_selector_all("a").unwrap_or_default();
        for link_id in links {
            if let Some(node) = dom.get_node(link_id) {
                let href = node.get_attribute("href").unwrap_or_default().to_string();
                let text = dom.text_content(link_id);
                let text = text.trim();
                let full_url = if href.starts_with("http://") || href.starts_with("https://") {
                    href.clone()
                } else if let Some(ref base) = base_url {
                    base.join(&href).map(|u| u.to_string()).unwrap_or(href.clone())
                } else {
                    href.clone()
                };
                if !full_url.is_empty() {
                    if text.is_empty() {
                        lines.push(full_url);
                    } else {
                        lines.push(format!("{}\t{}", full_url, text));
                    }
                }
            }
        }
    });
    lines.join("\n")
}

const MARKDOWN_JS: &str = r#"
(function() {
    var base = document.location ? document.location.href : '';
    function resolveUrl(u) {
        if (!u || u.match(/^(https?:|mailto:|javascript:|#|data:)/i)) return u;
        if (u.indexOf('//')=== 0) return 'https:' + u;
        try { return new URL(u, base).href; } catch(e) { return u; }
    }
    function toMd(el, depth) {
        if (!el) return '';
        if (el.nodeType === 3) {
            var t = el.textContent || '';
            if (!t.trim()) return t.indexOf('\n') >= 0 ? '\n' : ' ';
            return t;
        }
        if (el.nodeType !== 1) return '';
        var tag = (el.tagName || '').toLowerCase();
        if (tag === 'script' || tag === 'style' || tag === 'noscript' || tag === 'link' || tag === 'meta') return '';
        var children = '';
        var cn = el.childNodes || [];
        for (var i = 0; i < cn.length; i++) children += toMd(cn[i], depth);
        children = children.replace(/\n{3,}/g, '\n\n');
        if (!children.trim() && tag !== 'br' && tag !== 'hr' && tag !== 'img') return '';
        switch(tag) {
            case 'h1': return '\n# ' + children.trim() + '\n\n';
            case 'h2': return '\n## ' + children.trim() + '\n\n';
            case 'h3': return '\n### ' + children.trim() + '\n\n';
            case 'h4': return '\n#### ' + children.trim() + '\n\n';
            case 'h5': return '\n##### ' + children.trim() + '\n\n';
            case 'h6': return '\n###### ' + children.trim() + '\n\n';
            case 'p': return '\n' + children.trim() + '\n\n';
            case 'br': return '\n';
            case 'hr': return '\n---\n\n';
            case 'strong': case 'b': return '**' + children + '**';
            case 'em': case 'i': return '*' + children + '*';
            case 'code': return '`' + children + '`';
            case 'pre': return '\n```\n' + children + '\n```\n\n';
            case 'blockquote': return '\n> ' + children.trim().replace(/\n/g, '\n> ') + '\n\n';
            case 'a':
                var href = resolveUrl(el.getAttribute('href') || '');
                if (href && children.trim()) return '[' + children.trim() + '](' + href + ')';
                return children;
            case 'img':
                var src = resolveUrl(el.getAttribute('src') || '');
                var alt = el.getAttribute('alt') || '';
                if (src) return '![' + alt + '](' + src + ')';
                return '';
            case 'ul': case 'ol':
                return '\n' + children + '\n';
            case 'li':
                var parent = el.parentNode;
                var isOrdered = parent && parent.tagName && parent.tagName.toLowerCase() === 'ol';
                var bullet = isOrdered ? '1. ' : '- ';
                return bullet + children.trim() + '\n';
            case 'table': return '\n' + children + '\n';
            case 'thead': case 'tbody': case 'tfoot': return children;
            case 'tr':
                var cells = [];
                var tds = el.childNodes || [];
                for (var j = 0; j < tds.length; j++) {
                    if (tds[j].nodeType === 1) cells.push(toMd(tds[j], depth).trim());
                }
                return '| ' + cells.join(' | ') + ' |\n';
            case 'th': case 'td': return children;
            case 'div': case 'section': case 'article': case 'main': case 'aside': case 'nav': case 'header': case 'footer':
                return '\n' + children;
            case 'span': return children;
            default: return children;
        }
    }
    var body = document.body || document.documentElement;
    var md = toMd(body, 0);
    md = md.replace(/\n{3,}/g, '\n\n').trim();
    return md;
})()
"#;

fn dump_markdown(page: &mut Page) {
    println!("{}", get_markdown(page));
}

fn smart_extract(page: &mut Page) -> String {
    let result = page.evaluate(SMART_EXTRACT_JS);
    match result {
        serde_json::Value::String(s) if !s.trim().is_empty() => s,
        _ => get_markdown(page),
    }
}

const SMART_EXTRACT_JS: &str = r#"
(function() {
    // Phase 1: Remove noise elements
    var noiseSelectors = [
        'script', 'style', 'noscript', 'link', 'meta', 'svg', 'iframe',
        'nav', '[role="navigation"]', '[role="banner"]', '[role="contentinfo"]',
        '.cookie-banner', '.cookie-consent', '.cookie-notice', '#cookie-banner',
        '.gdpr', '.consent', '[class*="cookie"]', '[id*="cookie"]',
        '.popup', '.modal', '.overlay', '.dialog',
        '[class*="popup"]', '[class*="modal"]', '[class*="overlay"]',
        '.ad', '.ads', '.advertisement', '[class*="advert"]',
        '.social-share', '.share-buttons', '.social-links',
        '.newsletter', '.subscribe', '.signup',
        '.breadcrumb', '.breadcrumbs', '.pagination',
        '.sidebar', '#sidebar', '[role="complementary"]',
        'header:not(article header)', 'footer:not(article footer)',
    ];
    noiseSelectors.forEach(function(sel) {
        try {
            var els = document.querySelectorAll(sel);
            for (var i = 0; i < els.length; i++) els[i].remove();
        } catch(e) {}
    });

    // Phase 2: Find the content block by text density
    function textLength(el) {
        return (el.textContent || '').replace(/\s+/g, ' ').trim().length;
    }
    function linkLength(el) {
        var links = el.querySelectorAll('a');
        var total = 0;
        for (var i = 0; i < links.length; i++) total += textLength(links[i]);
        return total;
    }
    function score(el) {
        var text = textLength(el);
        if (text < 50) return 0;
        var link = linkLength(el);
        var linkDensity = text > 0 ? link / text : 1;
        if (linkDensity > 0.6) return 0;
        var paragraphs = el.querySelectorAll('p, li, td, blockquote, pre').length;
        return text * (1 - linkDensity) * (1 + Math.min(paragraphs, 10) * 0.1);
    }

    // Try semantic selectors first
    var contentSelectors = [
        'article', '[role="main"]', 'main',
        '.post-content', '.article-content', '.entry-content',
        '.content', '#content', '.post', '.article',
        '.mw-body-content', '.mw-parser-output',
        '#readme', '.markdown-body',
    ];

    var best = null;
    var bestScore = 0;

    for (var i = 0; i < contentSelectors.length; i++) {
        var el = document.querySelector(contentSelectors[i]);
        if (el) {
            var s = score(el);
            if (s > bestScore) {
                bestScore = s;
                best = el;
            }
        }
    }

    // Fallback: score all divs and sections
    if (!best || bestScore < 200) {
        var candidates = document.querySelectorAll('div, section, article');
        for (var i = 0; i < candidates.length; i++) {
            var s = score(candidates[i]);
            if (s > bestScore) {
                bestScore = s;
                best = candidates[i];
            }
        }
    }

    // Phase 3: Convert to markdown
    var target = best || document.body || document.documentElement;

    var base = document.location ? document.location.href : '';
    function resolveUrl(u) {
        if (!u || u.match(/^(https?:|mailto:|javascript:|#|data:)/i)) return u;
        if (u.indexOf('//')=== 0) return 'https:' + u;
        try { return new URL(u, base).href; } catch(e) { return u; }
    }

    function toMd(el, depth) {
        if (!el) return '';
        if (el.nodeType === 3) {
            var t = el.textContent || '';
            if (!t.trim()) return t.indexOf('\n') >= 0 ? '\n' : ' ';
            return t;
        }
        if (el.nodeType !== 1) return '';
        var tag = (el.tagName || '').toLowerCase();
        if (tag === 'script' || tag === 'style' || tag === 'noscript') return '';
        var children = '';
        var cn = el.childNodes || [];
        for (var i = 0; i < cn.length; i++) children += toMd(cn[i], depth);
        children = children.replace(/\n{3,}/g, '\n\n');
        if (!children.trim() && tag !== 'br' && tag !== 'hr' && tag !== 'img') return '';
        switch(tag) {
            case 'h1': return '\n# ' + children.trim() + '\n\n';
            case 'h2': return '\n## ' + children.trim() + '\n\n';
            case 'h3': return '\n### ' + children.trim() + '\n\n';
            case 'h4': return '\n#### ' + children.trim() + '\n\n';
            case 'h5': return '\n##### ' + children.trim() + '\n\n';
            case 'h6': return '\n###### ' + children.trim() + '\n\n';
            case 'p': return '\n' + children.trim() + '\n\n';
            case 'br': return '\n';
            case 'hr': return '\n---\n\n';
            case 'strong': case 'b': return '**' + children + '**';
            case 'em': case 'i': return '*' + children + '*';
            case 'code': return '`' + children + '`';
            case 'pre': return '\n```\n' + children + '\n```\n\n';
            case 'blockquote': return '\n> ' + children.trim().replace(/\n/g, '\n> ') + '\n\n';
            case 'a':
                var href = resolveUrl(el.getAttribute('href') || '');
                if (href && children.trim()) return '[' + children.trim() + '](' + href + ')';
                return children;
            case 'img':
                var src = resolveUrl(el.getAttribute('src') || '');
                var alt = el.getAttribute('alt') || '';
                if (src) return '![' + alt + '](' + src + ')';
                return '';
            case 'ul': case 'ol': return '\n' + children + '\n';
            case 'li':
                var parent = el.parentNode;
                var isOrdered = parent && parent.tagName && parent.tagName.toLowerCase() === 'ol';
                var bullet = isOrdered ? '1. ' : '- ';
                return bullet + children.trim() + '\n';
            case 'table': return '\n' + children + '\n';
            case 'thead': case 'tbody': case 'tfoot': return children;
            case 'tr':
                var cells = [];
                var tds = el.childNodes || [];
                for (var j = 0; j < tds.length; j++) {
                    if (tds[j].nodeType === 1) cells.push(toMd(tds[j], depth).trim());
                }
                return '| ' + cells.join(' | ') + ' |\n';
            case 'th': case 'td': return children;
            case 'script': case 'style': case 'noscript': return '';
            case 'div': case 'section': case 'article': case 'main': case 'aside':
            case 'header': case 'footer': case 'figure': case 'figcaption':
                return '\n' + children;
            case 'span': return children;
            default: return children;
        }
    }

    var md = toMd(target, 0);
    md = md.replace(/\n{3,}/g, '\n\n').trim();
    return md;
})()
"#;

fn dump_links(page: &Page) {
    let base_url = page.url.clone();
    page.with_dom(|dom| {
        let links = dom.query_selector_all("a").unwrap_or_default();
        for link_id in links {
            if let Some(node) = dom.get_node(link_id) {
                let href = node.get_attribute("href").unwrap_or_default().to_string();
                let text = dom.text_content(link_id);
                let text = text.trim();

                let full_url = if href.starts_with("http://") || href.starts_with("https://") {
                    href.clone()
                } else if let Some(ref base) = base_url {
                    base.join(&href).map(|u| u.to_string()).unwrap_or(href.clone())
                } else {
                    href.clone()
                };

                if !full_url.is_empty() {
                    if text.is_empty() {
                        println!("{}", full_url);
                    } else {
                        println!("{}\t{}", full_url, text);
                    }
                }
            }
        }
    });
}