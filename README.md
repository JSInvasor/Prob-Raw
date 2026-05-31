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
prob-raw <url> <saniye> <threads>
```

| Argüman   | Açıklama                                            |
|-----------|-----------------------------------------------------|
| `url`     | Hedef adres (örn. `https://example.com`)            |
| `saniye`  | Test süresi (saniye)                                |
| `threads` | Eşzamanlı worker / in-flight istek sayısı           |

### Örnek

```bash
./prob-raw https://example.com 30 200
```

30 saniye boyunca 200 eşzamanlı worker ile istek basar.

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
