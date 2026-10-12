//! Local-only download baseline. Reports timings; it does not enforce throughput gates.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use harvester_engine::{EngineConfig, EngineEvent, EngineHandle, JobTiming, UrlPolicy};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut sites = 4_usize;
    let mut jobs = 12_usize;
    let mut latency_ms = 50_u64;
    let mut body_bytes = 8192_usize;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--help" {
            println!(
                "download_bench [--sites K] [--jobs N] [--latency-ms MS] [--body-bytes BYTES]"
            );
            return Ok(());
        }
        let value = args.next().ok_or("missing benchmark option value")?;
        match flag.as_str() {
            "--sites" => sites = value.parse()?,
            "--jobs" => jobs = value.parse()?,
            "--latency-ms" => latency_ms = value.parse()?,
            "--body-bytes" => body_bytes = value.parse()?,
            _ => return Err(format!("unknown option {flag}").into()),
        }
    }
    if sites == 0 || jobs == 0 || body_bytes < 256 {
        return Err("sites and jobs must be positive; body size must be at least 256 bytes".into());
    }
    let prefix = "<html><head><title>Download baseline</title></head><body><article><h1>Download baseline</h1><p>";
    let suffix = "</p></article></body></html>";
    let prose = "Local benchmark article with detailed reporting and useful context. ";
    let fill = body_bytes - prefix.len() - suffix.len();
    let body = format!(
        "{prefix}{}{suffix}",
        prose.repeat(fill.div_ceil(prose.len()))[..fill].to_owned()
    );
    let body = Arc::new(body);
    let running = Arc::new(AtomicBool::new(true));
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let mut urls = Vec::new();
    let mut servers = Vec::new();
    for _ in 0..sites {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        urls.push(format!("http://{}", listener.local_addr()?));
        let running = Arc::clone(&running);
        let active = Arc::clone(&active);
        let maximum = Arc::clone(&maximum);
        let body = Arc::clone(&body);
        servers.push(thread::spawn(move || {
            let mut requests = Vec::new();
            while running.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let active = Arc::clone(&active);
                        let maximum = Arc::clone(&maximum);
                        let body = Arc::clone(&body);
                        requests.push(thread::spawn(move || {
                            serve_request(stream, &body, &active, &maximum, latency_ms)
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => {
                        eprintln!("stub accept failed: {error}");
                        break;
                    }
                }
            }
            for request in requests {
                if request.join().is_err() {
                    eprintln!("stub request thread panicked");
                }
            }
        }));
    }
    let output = tempfile::tempdir()?;
    let mut config = EngineConfig::default_with_output(output.path().to_path_buf());
    config.url_policy = UrlPolicy {
        block_private_ips: false,
        ..UrlPolicy::default()
    };
    let engine = EngineHandle::new(config);
    let started = Instant::now();
    for job in 0..jobs {
        engine.enqueue(
            job as u64 + 1,
            format!("{}/article-{job}", urls[job % sites]),
        );
    }
    let mut finished = 0;
    let mut failed = 0;
    let mut sums = JobTiming::default();
    let mut timed_out = false;
    let deadline = started
        + Duration::from_secs(120)
        + Duration::from_millis(latency_ms.saturating_mul(jobs as u64));
    while finished < jobs {
        if Instant::now() > deadline {
            timed_out = true;
            break;
        }
        while let Some(event) = engine.try_recv() {
            if let EngineEvent::JobCompleted { timing, result, .. } = event {
                finished += 1;
                failed += usize::from(result.is_err());
                sums.queue_wait += timing.queue_wait;
                sums.fetch += timing.fetch;
                sums.fetch_attempts += timing.fetch_attempts;
                sums.decode += timing.decode;
                sums.extract += timing.extract;
                sums.tokenize += timing.tokenize;
                sums.document_build += timing.document_build;
                sums.article_write += timing.article_write;
                sums.marker_write += timing.marker_write;
                sums.total += timing.total;
                sums.bytes += timing.bytes;
            }
        }
        if finished < jobs {
            thread::sleep(Duration::from_millis(2));
        }
    }
    let wall = started.elapsed();
    drop(engine);
    running.store(false, Ordering::Relaxed);
    for server in servers {
        if server.join().is_err() {
            eprintln!("stub server thread panicked");
        }
    }
    println!("download_bench sites={sites} jobs={jobs} latency_ms={latency_ms} body_bytes={body_bytes} wall_ms={} max_concurrency={} failures={failed}", wall.as_millis(), maximum.load(Ordering::Relaxed));
    println!("step_sums queue_wait_ms={} fetch_ms={} attempts={} decode_ms={} extract_ms={} tokenize_ms={} build_ms={} write_ms={} manifest_ms={} total_ms={} bytes={}", sums.queue_wait.as_millis(), sums.fetch.as_millis(), sums.fetch_attempts, sums.decode.as_millis(), sums.extract.as_millis(), sums.tokenize.as_millis(), sums.document_build.as_millis(), sums.article_write.as_millis(), sums.marker_write.as_millis(), sums.total.as_millis(), sums.bytes);
    if timed_out {
        return Err(
            format!("download benchmark timed out after {finished}/{jobs} completions").into(),
        );
    }
    if failed > 0 {
        return Err("download benchmark had failed jobs".into());
    }
    Ok(())
}

fn serve_request(
    mut stream: TcpStream,
    body: &str,
    active: &AtomicUsize,
    maximum: &AtomicUsize,
    latency_ms: u64,
) {
    // Accepted sockets inherit the listener's nonblocking mode on Windows.
    if let Err(error) = stream.set_nonblocking(false).and_then(|()| {
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))
    }) {
        eprintln!("stub socket setup failed: {error}");
        return;
    }
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let count = match stream.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                eprintln!("stub request read failed: {error}");
                return;
            }
        };
        if count == 0 {
            return;
        }
        request.extend_from_slice(&buffer[..count]);
    }
    let in_flight = active.fetch_add(1, Ordering::SeqCst) + 1;
    maximum.fetch_max(in_flight, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(latency_ms));
    if let Err(error) = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body) {
        eprintln!("stub response write failed: {error}");
    }
    active.fetch_sub(1, Ordering::SeqCst);
}
