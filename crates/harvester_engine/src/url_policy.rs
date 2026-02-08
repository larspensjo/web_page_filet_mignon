use std::fmt;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};

use url::Url;

/// Policy that describes which URLs are allowed.
#[derive(Debug, Clone)]
pub struct UrlPolicy {
    pub allowed_schemes: Vec<String>,
    pub block_private_ips: bool,
    pub allowed_hosts: Option<Vec<String>>,
    pub blocked_hosts: Vec<String>,
}

impl Default for UrlPolicy {
    fn default() -> Self {
        Self {
            allowed_schemes: vec!["http".to_string(), "https".to_string()],
            block_private_ips: true,
            allowed_hosts: None,
            blocked_hosts: Vec::new(),
        }
    }
}

impl UrlPolicy {
    /// Returns Ok(()) if the parsed URL satisfies the policy.
    pub fn check(&self, url: &Url) -> Result<(), UrlPolicyViolation> {
        let scheme = url.scheme();
        if !self
            .allowed_schemes
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(scheme))
        {
            return Err(UrlPolicyViolation::DisallowedScheme {
                scheme: scheme.to_string(),
            });
        }

        let host = url
            .host_str()
            .ok_or_else(|| UrlPolicyViolation::DnsResolutionFailed {
                host: "<empty>".to_string(),
            })?
            .to_string();
        let normalized_host = host.to_lowercase();

        if self
            .blocked_hosts
            .iter()
            .any(|blocked| blocked.eq_ignore_ascii_case(&normalized_host))
        {
            return Err(UrlPolicyViolation::BlockedHost {
                host: normalized_host.clone(),
            });
        }

        let mut allowed_by_list = false;
        if let Some(ref allowed_hosts) = self.allowed_hosts {
            if allowed_hosts
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(&normalized_host))
            {
                allowed_by_list = true;
            } else {
                return Err(UrlPolicyViolation::BlockedHost {
                    host: normalized_host.clone(),
                });
            }
        }

        if self.block_private_ips && !allowed_by_list {
            let port = url.port_or_known_default().unwrap_or(80);
            let mut resolved_any = false;
            let addrs = (normalized_host.as_str(), port)
                .to_socket_addrs()
                .map_err(|_| UrlPolicyViolation::DnsResolutionFailed {
                    host: normalized_host.clone(),
                })?;
            for socket in addrs {
                resolved_any = true;
                let ip = socket.ip();
                if is_private_ip(&ip) {
                    return Err(UrlPolicyViolation::PrivateIp {
                        host: normalized_host.clone(),
                        ip,
                    });
                }
            }
            if !resolved_any {
                return Err(UrlPolicyViolation::NoPublicIp {
                    host: normalized_host.clone(),
                });
            }
        }

        Ok(())
    }
}

fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_ipv4(v4),
        IpAddr::V6(v6) => is_private_ipv6(v6),
    }
}

fn is_private_ipv4(ip: &Ipv4Addr) -> bool {
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || matches!(ip.octets()[0], 100) && (ip.octets()[1] & 0b1100_0000) == 0b0100_0000
}

fn is_private_ipv6(ip: &std::net::Ipv6Addr) -> bool {
    ip.is_loopback() || ip.is_unspecified() || ip.is_unique_local() || ip.is_unicast_link_local()
}

/// Represents a specific policy violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlPolicyViolation {
    DisallowedScheme { scheme: String },
    PrivateIp { host: String, ip: IpAddr },
    BlockedHost { host: String },
    DnsResolutionFailed { host: String },
    NoPublicIp { host: String },
}

impl fmt::Display for UrlPolicyViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UrlPolicyViolation::DisallowedScheme { scheme } => {
                write!(f, "scheme '{}' is not allowed", scheme)
            }
            UrlPolicyViolation::PrivateIp { host, ip } => {
                write!(f, "host '{}' resolved to private ip {}", host, ip)
            }
            UrlPolicyViolation::BlockedHost { host } => write!(f, "host '{}' is blocked", host),
            UrlPolicyViolation::DnsResolutionFailed { host } => {
                write!(f, "dns resolution failed for '{}'", host)
            }
            UrlPolicyViolation::NoPublicIp { host } => {
                write!(f, "host '{}' did not resolve to a public ip", host)
            }
        }
    }
}
