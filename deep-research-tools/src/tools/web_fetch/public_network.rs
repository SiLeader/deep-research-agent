use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub(super) struct PublicDns;

impl Resolve for PublicDns {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            validate_addresses(&addresses)?;
            // The connector uses these validated addresses directly; there is no second lookup.
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

fn validate_addresses(addresses: &[SocketAddr]) -> std::io::Result<()> {
    if addresses.is_empty() || addresses.iter().any(|address| !is_public(address.ip())) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "fetch requires public IP addresses",
        ));
    }
    Ok(())
}

pub(super) fn validate_url(url: &reqwest::Url) -> anyhow::Result<()> {
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https"),
        "fetch requires HTTP or HTTPS"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "fetch URL must not contain credentials"
    );
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("fetch URL requires a host"))?;
    if let Ok(ip) = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
    {
        anyhow::ensure!(is_public(ip), "fetch requires a public IP address");
    }
    Ok(())
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let value = u32::from(ip);
    let excluded = [
        (0x00000000, 8),
        (0x0a000000, 8),
        (0x64400000, 10),
        (0x7f000000, 8),
        (0xa9fe0000, 16),
        (0xac100000, 12),
        (0xc0000000, 24),
        (0xc0000200, 24),
        (0xc0586300, 24),
        (0xc0a80000, 16),
        (0xc6120000, 15),
        (0xc6336400, 24),
        (0xcb007100, 24),
        (0xe0000000, 3),
    ];
    !excluded
        .iter()
        .any(|&(network, prefix)| value & (u32::MAX << (32 - prefix)) == network)
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => {
            // Permit global unicast only. Exclude special-purpose and transition ranges,
            // including IPv4-mapped addresses, Teredo, 6to4 and documentation networks.
            let segments = ip.segments();
            segments[0] & 0xe000 == 0x2000
                && !(segments[0] == 0x2001 && segments[1] < 0x0200)
                && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
                && segments[0] != 0x2002
                && !(segments[0] == 0x3fff && segments[1] < 0x1000)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_nonpublic_addresses_and_alternative_encodings() {
        for host in [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.168.1.1",
            "192.0.2.1",
            "198.18.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "[::]",
            "[::1]",
            "[::ffff:127.0.0.1]",
            "[fc00::1]",
            "[fe80::1]",
            "[ff02::1]",
            "[2001:db8::1]",
            "[2002:7f00:1::1]",
            "[2001::1]",
            "[3fff::1]",
            "2130706433",
            "0x7f000001",
            "127.1",
        ] {
            assert!(
                validate_url(&reqwest::Url::parse(&format!("http://{host}/")).unwrap()).is_err(),
                "{host}"
            );
        }
        for url in [
            "https://8.8.8.8/",
            "https://[2606:4700:4700::1111]/",
            "https://example.com/",
        ] {
            validate_url(&reqwest::Url::parse(url).unwrap()).unwrap();
        }
        assert!(validate_url(&reqwest::Url::parse("file:///etc/passwd").unwrap()).is_err());
        assert!(
            validate_url(&reqwest::Url::parse("http://user:pass@example.com/").unwrap()).is_err()
        );
    }

    #[test]
    fn rejects_mixed_public_private_dns_answers() {
        assert!(validate_addresses(&[]).is_err());
        assert!(
            validate_addresses(&["8.8.8.8:0".parse().unwrap(), "127.0.0.1:0".parse().unwrap()])
                .is_err()
        );
        validate_addresses(&["8.8.8.8:0".parse().unwrap()]).unwrap();
    }
}
