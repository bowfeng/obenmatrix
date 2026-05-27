//! URL safety — SSRF protection and hostname validation.

use std::io;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::sync::OnceLock;

use once_cell::sync::Lazy;

/// Hostnames always blocked regardless of config.
static ALWAYS_BLOCKED_HOSTNAMES: Lazy<Vec<&'static str>> = Lazy::new(|| {
    vec![
        "metadata.google.internal",
        "metadata.goog",
    ]
});

/// Hosts allowed to resolve to private IPs (HTTPS only bypass).
static TRUSTED_PRIVATE_IP_HOSTS: Lazy<Vec<&'static str>> = Lazy::new(|| {
    vec!["multimedia.nt.qq.com.cn"]
});

/// Check if the host is always blocked (cloud metadata endpoints).
pub fn is_always_blocked_host(host: &str) -> bool {
    ALWAYS_BLOCKED_HOSTNAMES.iter().any(|blocked| host == *blocked)
}

/// Check if a URL is safe to fetch (SSRF protection).
pub fn is_safe_url(url_str: &str) -> io::Result<bool> {
    let parsed = url::Url::parse(url_str)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;

    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Ok(false),
    }

    let host = match parsed.host_str() {
        Some(h) if !h.is_empty() => h,
        _ => return Ok(false),
    };

    if is_always_blocked_host(host) {
        return Ok(false);
    }

    let allow_private = {
        static ALLOC: OnceLock<bool> = OnceLock::new();
        *ALLOC.get_or_init(|| {
            std::env::var("HERMES_ALLOW_PRIVATE_URLS")
                .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(false)
        })
    };

    let addrs = match format!("{}:443", host).to_socket_addrs() {
        Ok(a) => a.collect::<Vec<_>>(),
        Err(_) => return Ok(false),
    };

    if addrs.is_empty() {
        return Ok(false);
    }

    for addr in addrs {
        ip_check(addr.ip(), host, allow_private, url_str.starts_with("https://"))?;
    }

    Ok(true)
}

pub fn is_quickly_blocked_url(url_str: &str) -> bool {
    match url::Url::parse(url_str) {
        Ok(parsed) => match parsed.host_str() {
            Some(host) => is_always_blocked_host(host),
            None => true,
        },
        Err(_) => true,
    }
}

fn ip_to_u32(ip: IpAddr) -> Option<u32> {
    match ip {
        IpAddr::V4(v4) => Some(u32::from(v4)),
        IpAddr::V6(v6) => v6.to_ipv4().map(u32::from),
    }
}

fn ip_check(ip: IpAddr, host: &str, allow_private: bool, is_https: bool) -> io::Result<()> {
    let ip_val = ip_to_u32(ip);

    // Trusted host bypass for HTTPS
    if is_https
        && TRUSTED_PRIVATE_IP_HOSTS
            .iter()
            .any(|trusted| host == *trusted)
    {
        return Ok(());
    }

    // Always-blocked networks
    if is_in_always_blocked_networks(ip) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("URL blocked: {} resolves to {}", host, ip),
        ));
    }

    if !allow_private {
        if is_private_or_special(ip, ip_val) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("URL blocked: {} resolves to private IP {}", host, ip),
            ));
        }
    }

    Ok(())
}

fn is_private_or_special(ip: IpAddr, ip_val: Option<u32>) -> bool {
    if ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
    {
        return true;
    }

    if let Some(val) = ip_val {
        // CGNAT 100.64.0.0/10
        if ip.is_ipv4() && val & 0xFFFF0000 == 0x64000000 {
            return true;
        }
        // Private ranges: 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, link-local 169.254.0.0/16
        if ip.is_ipv4() {
            let octets = [
                (val >> 24) as u8,
                (val >> 16) as u8,
                (val >> 8) as u8,
                val as u8,
            ];
            // 10.0.0.0/8
            if octets[0] == 10 {
                return true;
            }
            // 172.16.0.0/12
            if octets[0] == 172 && (octets[1] & 0xF0) == 16 {
                return true;
            }
            // 192.168.0.0/16
            if octets[0] == 192 && octets[1] == 168 {
                return true;
            }
            // 169.254.0.0/16 (link-local)
            if octets[0] == 169 && octets[1] == 254 {
                return true;
            }
        }
    }

    false
}

fn is_in_always_blocked_networks(ip: IpAddr) -> bool {
    let blocked_ips: [u32; 4] = [
        // AWS (169.254.169.254)
        u32::from(Ipv4Addr::new(169, 254, 169, 254)),
        // GCP (169.254.170.2)
        u32::from(Ipv4Addr::new(169, 254, 170, 2)),
        // Azure (168.63.129.16)
        u32::from(Ipv4Addr::new(168, 63, 129, 16)),
        // Alicloud (100.100.2.146)
        u32::from(Ipv4Addr::new(100, 100, 2, 146)),
    ];

    if let Some(val) = ip_to_u32(ip) {
        blocked_ips.contains(&val)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_public_url() {
        let result = is_safe_url("https://example.com/api");
        assert!(result.is_ok());
    }

    #[test]
    fn test_unsafe_foo_scheme() {
        let result = is_safe_url("ftp://example.org/file");
        assert!(matches!(result, Ok(false)));
    }

    #[test]
    fn test_blocked_metadata_host() {
        let result = is_safe_url("http://metadata.google.internal/computeMetadata/v1/");
        assert!(matches!(result, Ok(false)));
    }

    #[test]
    #[allow(clippy::empty_docs)]
    fn test_invalid_url_rejected() {
        let result = is_safe_url("not a url");
        assert!(result.is_err());
    }

    #[test]
    fn test_always_blocked_metadata() {
        assert!(is_quickly_blocked_url("http://metadata.google.internal/"));
        assert!(!is_quickly_blocked_url("https://example.com"));
    }
}
