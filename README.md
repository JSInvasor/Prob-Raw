# prob-raw

Rust ile yazılmış, yüksek performanslı **HTTP/2 yük testi / benchmark** aracı.
`tokio` + `reqwest` (rustls, h2 multiplexing) üzerine kuruludur.

## ⚠️ Sorumlu kullanım

Bu araç yalnızca **sahibi olduğun ya da yük testi için açıkça izinli olduğun**
sistemlerde kullanılmalıdır. İzinsiz hedeflere yüksek hacimli trafik basmak
DoS saldırısı sayılır ve çoğu ülkede yasa dışıdır. `wrk`, `bombardier`, `oha`
gibi araçlarla aynı kategoridedir.

## Derleme

```bash
cargo build --release
```

Çalıştırılabilir dosya: `target/release/prob-raw`
(kolaylık için `cp target/release/prob-raw ./prob-raw` yapabilirsin)

## Kullanım

```
prob-raw <url> <saniye> <threads> [connections]
```

| Argüman         | Açıklama                                                  |
|-----------------|----------------------------------------------------------|
| `url`           | Hedef adres (örn. `https://example.com`)                  |
| `saniye`        | Test süresi (saniye)                                      |
| `threads`       | Eşzamanlı worker / in-flight istek sayısı                 |
| `connections`   | (opsiyonel) Bağımsız HTTP/2 bağlantı sayısı — varsayılan otomatik |

### Örnek

```bash
./prob-raw https://example.com 30 1000 16
```

30 saniye boyunca 1000 eşzamanlı worker'ı 16 bağımsız HTTP/2 bağlantısına
dağıtarak istek basar.

## Yüksek throughput (170k+ req/s) ipuçları

HTTP/2'de reqwest bir host'a **tek bağlantı** açıp multiplex yapar. Sunucunun
`MAX_CONCURRENT_STREAMS` limiti (genelde 100–250) yüzünden tek bağlantı
üzerinden in-flight istek sınırlıdır — bu yüzden `threads`'i artırmak tek
başına yetmez. Araç bu yüzden **birden fazla bağımsız bağlantı** açar.

- `connections` değerini elle yükselt: `threads`'i bağlantılara böleriz, bağlantı
  başına ~100–256 akış ideal. Örn. `... 2000 20` → bağlantı başına 100 akış.
- `threads`'i kademeli artır (500 → 1000 → 2000) ve doygunluğu gözle.
- Ölçen makinenin CPU/çekirdek sayısı ve ağı belirleyicidir; 170k req/s için
  genelde çok çekirdekli bir makine + düşük gecikmeli (aynı bölge/LAN) hedef gerekir.
- `ulimit -n` (açık dosya/soket limiti) yüksek olmalı: `ulimit -n 1000000`.
- Hedefin gerçekten HTTP/2 verdiğini doğrula; HTTP/1.1'e düşerse throughput
  bağlantı başına tek isteğe iner ve bu rakamlara ulaşılamaz.

## Çıktı

- Saniyelik canlı `req/s` raporu
- Toplam istek, başarılı/hatalı sayısı
- Ortalama req/s ve veri hacmi (MB/s)
- Gecikme dağılımı: min / ort / p50 / p90 / p99 / max (ms)

`Ctrl-C` ile testi erken durdurabilirsin; özet yine yazdırılır.

## Notlar

- Protokol ALPN ile müzakere edilir. Sunucu HTTP/2 desteklemiyorsa araç
  uyarı verir ve müzakere edilen sürümle (örn. HTTP/1.1) devam eder.
- HTTP/2 multiplexing sayesinde az sayıda TCP bağlantısı üzerinden çok
  sayıda eşzamanlı istek akar; `threads` değeri in-flight istek sayısını belirler.
- Gerçek darboğaz çoğu zaman ağ gecikmesi, TLS el sıkışması veya istemci
  CPU'sudur. `threads` değerini kademeli artırarak sistemin tepkisini ölç.
