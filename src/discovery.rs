//! Receiver discovery (mDNS `_airplay._tcp` via Nyx `discovery`), lookup by name, and a
//! read-only `GET /info` to learn a receiver's current volume.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use ::discovery::{DeviceCache, DiscoveredDevice, DiscoveryOptions, discover_devices};
use anyhow::{Context, anyhow, bail};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tracing::{debug, info};

use crate::settings::{app_dir, receiver_cache_path};

pub const BROWSE_TIME: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq)]
pub struct Receiver {
    pub name: String,
    pub addr: SocketAddr,
    pub model: Option<String>,
}

fn is_airplay(dev: &DiscoveredDevice) -> bool {
    dev.instance_name.contains("._airplay._tcp")
}

fn to_receiver(dev: &DiscoveredDevice) -> Receiver {
    Receiver {
        name: dev.name.clone(),
        addr: SocketAddr::from((dev.ip, dev.port)),
        model: dev
            .capabilities
            .model
            .clone()
            .or_else(|| dev.txt_records.get("model").cloned()),
    }
}

/// Browses for AirPlay 2 receivers for [`BROWSE_TIME`] on the LAN interfaces (see
/// [`lan_interfaces`]), or only on `interface`. Updates our own cache file.
pub async fn discover(interface: Option<String>) -> anyhow::Result<Vec<Receiver>> {
    let found = match interface {
        Some(name) => browse(Some(name), receiver_cache_path()).await?,
        None => {
            let lan = lan_interfaces();
            match lan.as_slice() {
                // No LAN adapter recognised: let Nyx pick (it may warn about odd adapters).
                [] => browse(None, receiver_cache_path()).await?,
                [one] => browse(Some(one.clone()), receiver_cache_path()).await?,
                several => browse_several(several).await?,
            }
        }
    };
    let mut list: Vec<Receiver> = found
        .iter()
        .filter(|d| is_airplay(d))
        .map(to_receiver)
        .collect();
    list.sort_by_key(|r| r.name.to_lowercase());
    list.dedup_by(|a, b| a.name == b.name);
    debug!("Discovered {} AirPlay receiver(s)", list.len());
    Ok(list)
}

async fn browse(
    interface: Option<String>,
    cache_path: PathBuf,
) -> anyhow::Result<Vec<DiscoveredDevice>> {
    let opts = DiscoveryOptions {
        timeout: BROWSE_TIME,
        interface,
        cache_fallback: false,
        dump_txt: false,
        cache_path: Some(cache_path),
    };
    discover_devices(&opts)
        .await
        .map_err(|e| anyhow!("mDNS browse failed: {e}"))
}

/// One browse per LAN interface, all at once. Each writes a scratch cache (Nyx always
/// saves one), and the merged result goes into our cache here, so the writes cannot race.
async fn browse_several(ifaces: &[String]) -> anyhow::Result<Vec<DiscoveredDevice>> {
    let scratch: Vec<PathBuf> = (0..ifaces.len())
        .map(|i| app_dir().join(format!("mdns-scratch-{i}.toml")))
        .collect();
    let results = futures::future::join_all(
        ifaces
            .iter()
            .zip(&scratch)
            .map(|(name, path)| browse(Some(name.clone()), path.clone())),
    )
    .await;
    for p in &scratch {
        let _ = std::fs::remove_file(p);
    }
    let mut found = Vec::new();
    let mut first_err = None;
    for r in results {
        match r {
            Ok(list) => found.extend(list),
            Err(e) => {
                debug!("{e}");
                first_err.get_or_insert(e);
            }
        }
    }
    if found.is_empty()
        && let Some(e) = first_err
    {
        return Err(e);
    }
    if !found.is_empty() {
        let path = receiver_cache_path();
        let mut cache = DeviceCache::load_from_path(&path).unwrap_or_default();
        cache.update(&found);
        if let Err(e) = cache.save_to_path(&path) {
            debug!("Could not update {}: {e}", path.display());
        }
    }
    Ok(found)
}

/// An address on a real LAN: not link-local (169.254.x.x, an adapter without DHCP),
/// loopback, unspecified, multicast or broadcast.
fn is_lan_address(ip: Ipv4Addr) -> bool {
    !(ip.is_link_local()
        || ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast())
}

/// Interfaces to send mDNS queries on: up, physical (Nyx's classification: no vEthernet,
/// VPN, tunnel...) and with a LAN IPv4 address. Nyx itself would also try adapters that only
/// have a link-local address; on Windows each of those fails with `os error 10049` and logs
/// a warning on every browse. Returns system interface names (adapter GUIDs on Windows).
pub fn lan_interfaces() -> Vec<String> {
    let Ok(classified) =
        netutil::enumerate_and_classify_interfaces(&netutil::ClassificationConfig::default())
    else {
        return Vec::new();
    };
    let mut chosen = Vec::new();
    let mut skipped = Vec::new();
    for c in &classified {
        let ip = c.primary_ipv4();
        let label = format!(
            "{} ({})",
            c.display_name(),
            ip.map(|i| i.to_string()).unwrap_or_default()
        );
        if c.is_usable_physical && c.iface.is_up && ip.is_some_and(is_lan_address) {
            chosen.push((c.iface.name.clone(), label));
        } else if c.is_usable_physical {
            skipped.push(label);
        }
    }
    // Log the choice once, and again only if it changes (browses run every 30 s).
    static LAST: Mutex<Option<String>> = Mutex::new(None);
    let summary = format!(
        "mDNS on {:?}; skipped (down, link-local or no LAN address): {skipped:?}",
        chosen.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>()
    );
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_deref() != Some(summary.as_str()) {
        info!("{summary}");
        *last = Some(summary);
    }
    chosen.into_iter().map(|(name, _)| name).collect()
}

/// Receivers remembered from earlier browses (our cache file), possibly stale.
pub fn cached() -> Vec<Receiver> {
    let cache = DeviceCache::load_from_path(&receiver_cache_path()).unwrap_or_default();
    let mut list: Vec<Receiver> = cache
        .devices
        .iter()
        .filter(|d| is_airplay(d))
        .map(to_receiver)
        .collect();
    list.dedup_by(|a, b| a.name == b.name);
    list
}

/// Resolves a receiver given as IP, IP:port, or name: mDNS first (IP addresses can
/// change), then our cache.
pub async fn resolve(target: &str, interface: Option<String>) -> anyhow::Result<Receiver> {
    if let Ok((ip, port)) = ::discovery::parse_target_addr(target) {
        return Ok(Receiver {
            name: ip.to_string(),
            addr: SocketAddr::from((ip, port)),
            model: None,
        });
    }
    let matches = |name: &str| name.eq_ignore_ascii_case(target);
    match discover(interface).await {
        Ok(list) => {
            if let Some(r) = list.iter().find(|r| matches(&r.name)) {
                return Ok(r.clone());
            }
        }
        Err(e) => info!("{e}; trying the receiver cache"),
    }
    let cache = DeviceCache::load_from_path(&receiver_cache_path()).unwrap_or_default();
    if let Some(dev) = cache
        .devices
        .iter()
        .find(|d| is_airplay(d) && matches(&d.name))
    {
        info!(
            "Receiver \"{target}\" not seen on mDNS; using cached address {}",
            dev.ip
        );
        return Ok(to_receiver(dev));
    }
    bail!("Receiver \"{target}\" not found. Is it powered on and on this network?")
}

/// Reads the receiver's current volume (0-100 %) from `GET /info` `initialVolume` (dB).
/// Read-only; plays nothing.
pub async fn read_volume_pct(addr: SocketAddr) -> anyhow::Result<f32> {
    let info = tokio::time::timeout(Duration::from_secs(3), get_info(addr))
        .await
        .map_err(|_| anyhow!("GET /info timed out"))??;
    volume_pct_from_info(&info)
}

/// `initialVolume` (dB) as 0-100 %. NaN or infinity counts as unknown: NaN would survive
/// every clamp and end up as 0 dB, full volume.
fn volume_pct_from_info(info: &plist::Dictionary) -> anyhow::Result<f32> {
    let db = info
        .get("initialVolume")
        .and_then(|v| {
            v.as_real()
                .or_else(|| v.as_signed_integer().map(|i| i as f64))
        })
        .ok_or_else(|| anyhow!("no initialVolume in /info"))?;
    if !db.is_finite() {
        bail!("/info initialVolume is not a number ({db})");
    }
    Ok(raop::volume::db_to_pct(db as f32))
}

/// The largest `/info` body we accept. Real ones are a few KiB; the cap keeps a host that
/// claims a huge Content-Length from making us buffer it.
const MAX_INFO_BODY: usize = 256 * 1024;

async fn get_info(addr: SocketAddr) -> anyhow::Result<plist::Dictionary> {
    let mut s = tokio::net::TcpStream::connect(addr)
        .await
        .with_context(|| format!("connect {addr}"))?;
    s.write_all(b"GET /info RTSP/1.0\r\nCSeq: 1\r\nUser-Agent: AirPlay/550.10\r\n\r\n")
        .await?;
    read_info_response(&mut s).await
}

/// Reads and parses a `/info` response. Anything malformed is an error, never a panic: the
/// other end is any host on the LAN that advertises `_airplay._tcp`.
async fn read_info_response<R: AsyncRead + Unpin>(s: &mut R) -> anyhow::Result<plist::Dictionary> {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    let header_end = loop {
        let n = s.read(&mut buf).await?;
        if n == 0 {
            bail!("connection closed before /info headers");
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(p) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        if data.len() > 64 * 1024 {
            bail!("/info headers too long");
        }
    };
    let head = String::from_utf8_lossy(&data[..header_end]).to_string();
    let status_ok = head.lines().next().is_some_and(|l| l.contains(" 200 "));
    if !status_ok {
        bail!("/info: {}", head.lines().next().unwrap_or("").trim());
    }
    let len = info_body_len(&head)?;
    let end = header_end
        .checked_add(len)
        .ok_or_else(|| anyhow!("/info Content-Length too large"))?;
    while data.len() < end {
        let n = s.read(&mut buf).await?;
        if n == 0 {
            bail!(
                "/info body truncated ({} of {len} bytes)",
                data.len() - header_end
            );
        }
        data.extend_from_slice(&buf[..n]);
    }
    match plist::from_bytes::<plist::Value>(&data[header_end..end])? {
        plist::Value::Dictionary(d) => Ok(d),
        _ => bail!("/info body is not a dictionary"),
    }
}

/// The body length from the `/info` response headers: required, and at most
/// [`MAX_INFO_BODY`].
fn info_body_len(head: &str) -> anyhow::Result<usize> {
    let value = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("content-length")
                .then_some(v.trim())
        })
        .ok_or_else(|| anyhow!("/info has no Content-Length"))?;
    let len: usize = value
        .parse()
        .map_err(|_| anyhow!("/info Content-Length is not a number: {value:?}"))?;
    if len > MAX_INFO_BODY {
        bail!("/info body too large ({len} bytes)");
    }
    Ok(len)
}

/// The local interface and address the session will bind to for `ip` (same rule as Nyx:
/// physical adapters only, subnet match first). For logging.
pub fn lan_interface_for(ip: Ipv4Addr) -> Option<(String, Ipv4Addr)> {
    let classified =
        netutil::enumerate_and_classify_interfaces(&netutil::ClassificationConfig::default())
            .ok()?;
    let sel = netutil::select_interface_for_target(&classified, ip)?;
    // Nyx binds only to an interface on the receiver's subnet; otherwise the OS routes.
    if !sel.iface.contains_ipv4(ip) || !sel.is_usable_physical {
        return None;
    }
    Some((sel.display_name().to_string(), sel.primary_ipv4()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_local_and_special_addresses_are_not_lan() {
        assert!(is_lan_address(Ipv4Addr::new(10, 0, 0, 20)));
        assert!(is_lan_address(Ipv4Addr::new(192, 168, 1, 20)));
        assert!(!is_lan_address(Ipv4Addr::new(169, 254, 194, 28)));
        assert!(!is_lan_address(Ipv4Addr::LOCALHOST));
        assert!(!is_lan_address(Ipv4Addr::UNSPECIFIED));
        assert!(!is_lan_address(Ipv4Addr::new(224, 0, 0, 251)));
        assert!(!is_lan_address(Ipv4Addr::new(239, 255, 255, 250)));
        assert!(!is_lan_address(Ipv4Addr::BROADCAST));
    }

    const BODY: &str = r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>initialVolume</key><real>-15</real></dict></plist>"#;

    fn response(content_length: &str, body: &str) -> Vec<u8> {
        format!("RTSP/1.0 200 OK\r\nCSeq: 1\r\nContent-Length: {content_length}\r\n\r\n{body}")
            .into_bytes()
    }

    async fn parse(bytes: Vec<u8>) -> anyhow::Result<plist::Dictionary> {
        read_info_response(&mut bytes.as_slice()).await
    }

    #[tokio::test]
    async fn info_with_a_matching_length_parses() {
        let info = parse(response(&BODY.len().to_string(), BODY))
            .await
            .unwrap();
        assert_eq!(volume_pct_from_info(&info).unwrap(), 50.0);
        // Bytes past the declared length are ignored.
        let extra = format!("{BODY}junk");
        assert!(
            parse(response(&BODY.len().to_string(), &extra))
                .await
                .is_ok()
        );
    }

    #[test]
    fn info_body_length_accepts_the_limit_and_rejects_larger_values() {
        assert_eq!(info_body_len("Content-Length: 262144\r\n").unwrap(), 262144);
        for length in ["262145".to_string(), usize::MAX.to_string()] {
            let error = info_body_len(&format!("Content-Length: {length}\r\n")).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("/info body too large ({length} bytes)")
            );
        }
    }

    #[test]
    fn info_body_length_accepts_lowercase_headers() {
        assert_eq!(info_body_len("content-length: 42\r\n").unwrap(), 42);
    }

    #[test]
    fn info_body_length_requires_a_length_header() {
        assert_eq!(
            info_body_len("RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n")
                .unwrap_err()
                .to_string(),
            "/info has no Content-Length"
        );
    }

    #[test]
    fn info_body_length_rejects_invalid_numbers() {
        for length in [
            "-1".to_string(),
            "12abc".to_string(),
            format!("{}0", usize::MAX),
        ] {
            let error = info_body_len(&format!("Content-Length: {length}\r\n")).unwrap_err();
            assert!(
                error
                    .to_string()
                    .starts_with("/info Content-Length is not a number:"),
                "{error}"
            );
        }
    }

    #[tokio::test]
    async fn oversized_info_is_rejected_before_reading_the_body() {
        use tokio::io::AsyncReadExt;

        let headers = "RTSP/1.0 200 OK\r\nContent-Length: 262145\r\n\r\n";
        let mut body = tokio::io::repeat(b'x').take(u64::MAX);
        let error = {
            let mut reader = headers.as_bytes().chain(&mut body);
            tokio::time::timeout(Duration::from_secs(1), read_info_response(&mut reader))
                .await
                .expect("oversized body should be rejected promptly")
                .unwrap_err()
        };
        assert_eq!(error.to_string(), "/info body too large (262145 bytes)");
        assert_eq!(body.limit(), u64::MAX, "the body must not be read");
    }

    #[tokio::test]
    async fn info_with_a_truncated_body_reports_the_missing_bytes() {
        let error = parse(response(&(BODY.len() + 10).to_string(), BODY))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "/info body truncated ({} of {} bytes)",
                BODY.len(),
                BODY.len() + 10
            )
        );
    }

    #[tokio::test]
    async fn info_with_a_non_success_status_is_rejected() {
        let bytes = response(&BODY.len().to_string(), BODY);
        let forbidden = String::from_utf8(bytes)
            .unwrap()
            .replacen("200 OK", "403 Forbidden", 1);
        assert_eq!(
            parse(forbidden.into_bytes()).await.unwrap_err().to_string(),
            "/info: RTSP/1.0 403 Forbidden"
        );
    }

    #[tokio::test]
    async fn info_headers_can_end_across_two_reads() {
        let prefix = format!("RTSP/1.0 200 OK\r\nContent-Length: {}\r\n\r", BODY.len());
        // Filling the duplex buffer forces the final newline into a later read.
        let (mut writer, mut reader) = tokio::io::duplex(prefix.len());
        let write = async {
            writer.write_all(prefix.as_bytes()).await.unwrap();
            writer
                .write_all(format!("\n{BODY}").as_bytes())
                .await
                .unwrap();
        };
        let (_, info) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(write, read_info_response(&mut reader))
        })
        .await
        .expect("split headers should finish parsing");
        assert_eq!(volume_pct_from_info(&info.unwrap()).unwrap(), 50.0);
    }

    #[tokio::test]
    async fn info_headers_over_the_limit_are_rejected() {
        let mut bytes = b"RTSP/1.0 200 OK\r\nX-Padding: ".to_vec();
        bytes.resize(64 * 1024 + 1, b'x');
        assert_eq!(
            parse(bytes).await.unwrap_err().to_string(),
            "/info headers too long"
        );
    }

    #[test]
    fn non_finite_initial_volume_is_unknown() {
        for db in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut info = plist::Dictionary::new();
            info.insert("initialVolume".into(), plist::Value::Real(db));
            assert!(volume_pct_from_info(&info).is_err(), "{db}");
        }
        let mut info = plist::Dictionary::new();
        info.insert("initialVolume".into(), plist::Value::Integer((-30).into()));
        assert_eq!(volume_pct_from_info(&info).unwrap(), 0.0);
    }
}
