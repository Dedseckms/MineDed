//! Free loopback port selection for mihomo's mixed-port and controller.

use std::net::{Ipv4Addr, TcpListener};

use lc_types::{AppError, AppResult};

/// Ports we never take even if free: Clash/mihomo defaults (a user's own
/// client may start later and expect them), DNS and privileged ports.
pub(crate) fn is_reserved(port: u16) -> bool {
    port < 1024 || matches!(port, 1053 | 9090 | 9097 | 7890..=7899)
}

/// Picks `n` distinct free, non-reserved ports from `candidates`, verifying
/// each by binding it on loopback (all listeners are held until the end so
/// the result is distinct). A port in use by anyone — e.g. the user's own
/// Clash — fails the bind and is skipped.
pub(crate) fn choose(candidates: impl IntoIterator<Item = u16>, n: usize) -> AppResult<Vec<u16>> {
    let mut held = Vec::with_capacity(n);
    let mut out = Vec::with_capacity(n);
    for port in candidates {
        if out.len() == n {
            break;
        }
        if is_reserved(port) || out.contains(&port) {
            continue;
        }
        if let Ok(l) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
            held.push(l);
            out.push(port);
        }
    }
    if out.len() < n {
        return Err(AppError::internal("no free local ports for the VPN"));
    }
    Ok(out)
}

/// `n` free ports chosen by the OS (ephemeral range).
pub(crate) fn pick_free(n: usize) -> AppResult<Vec<u16>> {
    let os_ports = std::iter::from_fn(|| {
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .ok()
    })
    .take(64);
    choose(os_ports, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_ports() {
        for p in [53, 80, 1053, 7890, 7895, 7899, 9090] {
            assert!(is_reserved(p), "{p}");
        }
        for p in [7889, 7900, 9091, 40000, 65535] {
            assert!(!is_reserved(p), "{p}");
        }
    }

    #[test]
    fn skips_reserved_taken_and_duplicates() {
        let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let taken_port = taken.local_addr().unwrap().port();
        let free = pick_free(2).unwrap();
        let got = choose([7890, 9090, 53, taken_port, free[0], free[0], free[1]], 2).unwrap();
        assert_eq!(got, free);
    }

    #[test]
    fn pick_free_gives_distinct_unreserved() {
        let p = pick_free(2).unwrap();
        assert_eq!(p.len(), 2);
        assert_ne!(p[0], p[1]);
        assert!(p.iter().all(|x| !is_reserved(*x)));
        assert!(choose([7890, 7891], 1).is_err());
    }
}
