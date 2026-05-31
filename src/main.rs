//! prob-raw — yüksek performanslı HTTP/2 yük testi / benchmark aracı
//!
//! Kullanım:
//!     prob-raw <url> <saniye> <threads> [connections]
//!
//! Örnek:
//!     prob-raw https://example.com 30 1000 16
//!
//! UYARI: Bu araç yalnızca sahibi olduğun ya da yük testi için açıkça
//! izinli olduğun sistemlerde kullanılmalıdır. İzinsiz hedeflere yüksek
//! hacimli trafik basmak DoS saldırısı sayılır ve yasa dışıdır.
//!
//! Performans notu (170k+ req/s için):
//! HTTP/2'de reqwest bir host'a TEK bağlantı açıp onun üzerinden multiplex
//! yapar. Sunucunun MAX_CONCURRENT_STREAMS limiti (genelde 100-250) yüzünden
//! tek bağlantı üzerinden in-flight istek sınırlıdır. Bu yüzden burada birden
//! fazla bağımsız bağlantı (`connections`) açıp worker'ları bunlara dağıtıyoruz.
//! Toplam eşzamanlılık = threads; bağlantı başına ~ threads/connections akış.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use reqwest::{Client, Url, Version};

/// Tüm worker'ların paylaştığı canlı sayaçlar.
struct Stats {
    success: AtomicU64,
    failed: AtomicU64,
    bytes: AtomicU64,
    /// Süre dolunca ya da Ctrl-C gelince false olur.
    running: AtomicBool,
}

impl Stats {
    fn new() -> Self {
        Stats {
            success: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            running: AtomicBool::new(true),
        }
    }
}

fn print_usage(prog: &str) {
    eprintln!("Kullanım: {prog} <url> <saniye> <threads> [connections]");
    eprintln!();
    eprintln!("  <url>          Hedef adres (örn. https://example.com)");
    eprintln!("  <saniye>       Test süresi, saniye cinsinden (örn. 30)");
    eprintln!("  <threads>      Eşzamanlı worker / in-flight istek sayısı (örn. 1000)");
    eprintln!("  [connections]  Bağımsız HTTP/2 bağlantı sayısı (varsayılan: otomatik)");
    eprintln!();
    eprintln!("Örnek: {prog} https://example.com 30 1000 16");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let prog = args
        .first()
        .map(|s| s.as_str())
        .unwrap_or("prob-raw")
        .to_string();

    if args.len() < 4 || args.len() > 5 {
        print_usage(&prog);
        std::process::exit(2);
    }

    let url = args[1].clone();
    let secs: u64 = match args[2].parse() {
        Ok(v) if v > 0 => v,
        _ => {
            eprintln!("Hata: <saniye> pozitif bir tam sayı olmalı.");
            std::process::exit(2);
        }
    };
    let threads: usize = match args[3].parse() {
        Ok(v) if v > 0 => v,
        _ => {
            eprintln!("Hata: <threads> pozitif bir tam sayı olmalı.");
            std::process::exit(2);
        }
    };

    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    // connections: kullanıcı vermezse, bağlantı başına ~256 akış olacak şekilde
    // otomatik seç (en az CPU çekirdek sayısı kadar). Bu sayede tek bağlantının
    // MAX_CONCURRENT_STREAMS limiti darboğaz olmaz.
    let connections: usize = if args.len() == 5 {
        match args[4].parse() {
            Ok(v) if v > 0 => v,
            _ => {
                eprintln!("Hata: [connections] pozitif bir tam sayı olmalı.");
                std::process::exit(2);
            }
        }
    } else {
        ((threads + 255) / 256).max(cpus)
    };
    let connections = connections.min(threads); // bağlantı sayısı worker'dan fazla olmasın

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(cpus)
        .enable_all()
        .build()
        .expect("tokio runtime kurulamadı");

    rt.block_on(run(url, secs, threads, connections, cpus));
}

/// Maksimum throughput için bağımsız bir HTTP/2 istemcisi kurar.
/// Her istemci kendi TCP/h2 bağlantısını açar; böylece tek bağlantının
/// akış limiti darboğaz olmaz.
fn build_client() -> Client {
    Client::builder()
        .user_agent("prob-raw/0.2")
        .http2_adaptive_window(true)
        // Akış ve bağlantı pencerelerini büyüterek küçük gövdelerde throughput artır.
        .http2_initial_stream_window_size(4 * 1024 * 1024)
        .http2_initial_connection_window_size(16 * 1024 * 1024)
        // Bağlantıyı canlı tut, boşta kapanmasın.
        .http2_keep_alive_interval(Duration::from_secs(10))
        .http2_keep_alive_timeout(Duration::from_secs(20))
        .http2_keep_alive_while_idle(true)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_nodelay(true)
        .build()
        .expect("HTTP istemcisi kurulamadı")
}

async fn run(url: String, secs: u64, threads: usize, connections: usize, cpus: usize) {
    let stats = Arc::new(Stats::new());

    // URL'i bir kez parse et; worker'lar her istekte yeniden parse etmesin.
    let parsed = match Url::parse(&url) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Geçersiz URL: {e}");
            std::process::exit(2);
        }
    };

    // Bağımsız istemci havuzu (her biri ayrı h2 bağlantısı).
    let clients: Vec<Client> = (0..connections).map(|_| build_client()).collect();

    // İlk istekle bağlantıyı ısıt ve protokol sürümünü öğren.
    match clients[0].get(parsed.clone()).send().await {
        Ok(resp) => {
            let ver = version_str(resp.version());
            println!("Hedef       : {url}");
            println!("Protokol    : {ver}");
            if resp.version() != Version::HTTP_2 {
                eprintln!(
                    "UYARI: Sunucu HTTP/2 müzakere etmedi ({ver}). Test bu protokolle \
                     devam edecek; HTTP/1.1'de bağlantı başına tek istek olacağı için \
                     throughput çok daha düşük olur."
                );
            }
        }
        Err(e) => {
            eprintln!("Hedefe bağlanılamadı: {e}");
            std::process::exit(1);
        }
    }

    println!("Süre        : {secs} sn");
    println!("Worker      : {threads}");
    println!("Bağlantı    : {connections}  (~{} akış/bağlantı)", threads / connections.max(1));
    println!("OS thread   : {cpus}");
    println!("------------------------------------------------------------");

    let start = Instant::now();
    let deadline = start + Duration::from_secs(secs);

    // Ctrl-C ile erken durdurma.
    {
        let stats = Arc::clone(&stats);
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                stats.running.store(false, Ordering::Relaxed);
            }
        });
    }

    // Saniyelik canlı rapor.
    let reporter = {
        let stats = Arc::clone(&stats);
        tokio::spawn(async move {
            let mut last = 0u64;
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            tick.tick().await; // ilk tick'i atla
            loop {
                tick.tick().await;
                if !stats.running.load(Ordering::Relaxed) {
                    break;
                }
                let total = stats.success.load(Ordering::Relaxed)
                    + stats.failed.load(Ordering::Relaxed);
                let rps = total.saturating_sub(last);
                last = total;
                let elapsed = start.elapsed().as_secs();
                println!(
                    "[{elapsed:>3}s] {rps:>9} req/s  | toplam: {total} | hata: {}",
                    stats.failed.load(Ordering::Relaxed)
                );
            }
        })
    };

    // Worker'ları başlat ve bağlantılara dağıt.
    let mut handles = Vec::with_capacity(threads);
    for i in 0..threads {
        let client = clients[i % connections].clone();
        let stats = Arc::clone(&stats);
        let url = parsed.clone();
        handles.push(tokio::spawn(
            async move { worker(client, url, deadline, stats).await },
        ));
    }

    // Süre dolunca durdur.
    {
        let stats = Arc::clone(&stats);
        tokio::spawn(async move {
            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
            stats.running.store(false, Ordering::Relaxed);
        });
    }

    // Worker histogramlarını topla.
    let mut hist = Histogram::<u64>::new(3).expect("histogram");
    for h in handles {
        if let Ok(local) = h.await {
            let _ = hist.add(&local);
        }
    }
    reporter.abort();

    let elapsed = start.elapsed();
    print_summary(&stats, &hist, elapsed);
}

/// Tek bir worker: deadline'a ya da durdurma sinyaline kadar istek basar.
async fn worker(
    client: Client,
    url: Url,
    deadline: Instant,
    stats: Arc<Stats>,
) -> Histogram<u64> {
    let mut hist = Histogram::<u64>::new(3).expect("histogram");

    while stats.running.load(Ordering::Relaxed) && Instant::now() < deadline {
        let t0 = Instant::now();
        match client.get(url.clone()).send().await {
            Ok(resp) => {
                // Gövdeyi drain et ki h2 akışı kapansın ve slot serbest kalsın.
                match resp.bytes().await {
                    Ok(body) => {
                        let micros = t0.elapsed().as_micros() as u64;
                        let _ = hist.record(micros);
                        stats.success.fetch_add(1, Ordering::Relaxed);
                        stats.bytes.fetch_add(body.len() as u64, Ordering::Relaxed);
                    }
                    Err(_) => {
                        stats.failed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            Err(_) => {
                stats.failed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    hist
}

fn version_str(v: Version) -> &'static str {
    match v {
        Version::HTTP_09 => "HTTP/0.9",
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_11 => "HTTP/1.1",
        Version::HTTP_2 => "HTTP/2",
        Version::HTTP_3 => "HTTP/3",
        _ => "bilinmiyor",
    }
}

fn print_summary(stats: &Stats, hist: &Histogram<u64>, elapsed: Duration) {
    let success = stats.success.load(Ordering::Relaxed);
    let failed = stats.failed.load(Ordering::Relaxed);
    let bytes = stats.bytes.load(Ordering::Relaxed);
    let total = success + failed;
    let secs = elapsed.as_secs_f64().max(1e-9);

    let rps = total as f64 / secs;
    let mb = bytes as f64 / (1024.0 * 1024.0);
    let mbps = mb / secs;

    println!("------------------------------------------------------------");
    println!("SONUÇ");
    println!("  Süre            : {:.2} sn", secs);
    println!("  Toplam istek    : {total}");
    println!("  Başarılı        : {success}");
    println!("  Hatalı          : {failed}");
    println!("  Ortalama req/s  : {:.0}", rps);
    println!("  Veri            : {:.2} MB ({:.2} MB/s)", mb, mbps);

    if hist.len() > 0 {
        let to_ms = |us: u64| us as f64 / 1000.0;
        println!("  Gecikme (ms):");
        println!("    min   : {:.2}", to_ms(hist.min()));
        println!("    ort   : {:.2}", hist.mean() / 1000.0);
        println!("    p50   : {:.2}", to_ms(hist.value_at_quantile(0.50)));
        println!("    p90   : {:.2}", to_ms(hist.value_at_quantile(0.90)));
        println!("    p99   : {:.2}", to_ms(hist.value_at_quantile(0.99)));
        println!("    max   : {:.2}", to_ms(hist.max()));
    }
    println!("------------------------------------------------------------");
}
