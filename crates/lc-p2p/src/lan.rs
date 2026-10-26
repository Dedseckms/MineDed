//! LAN world detection on the host and LAN announcements on the guest.
//!
//! Minecraft announces an open LAN world every ~1.5 s to `224.0.2.60:4445`
//! as `[MOTD]<text>[/MOTD][AD]<port>[/AD]`; the game log also prints the port
//! on the server thread ("Started serving on <port>"). Only sources on this
//! machine are trusted; the port is fixed once the room is created.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub(crate) const LAN_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 2, 60);
pub(crate) const LAN_PORT: u16 = 4445;
const ANNOUNCE_EVERY: Duration = Duration::from_millis(1500);

/// Extracts the LAN port from a game log line, if the line reports it.
///
/// Only the integrated server's own lines count (`[Server thread/INFO]`);
/// anything containing `[CHAT]` is ignored, so a player cannot redirect the
/// tunnel by typing "Started serving on 22" in chat.
pub fn port_from_log_line(line: &str) -> Option<u16> {
    const MARKERS: [&str; 2] = ["Started serving on ", "Local game hosted on port "];
    if line.contains("[CHAT]") {
        return None;
    }
    // Log4j prefix ends at the first "]: ": `[time] [Server thread/INFO]: msg`
    // (Forge adds a logger tag: `[time] [Server thread/INFO] [x.IntegratedServer/]: msg`).
    let end = line.find("]: ")?;
    let prefix = &line[..end + 1];
    let msg = &line[end + 3..];
    // `IntegratedServer.publishServer` runs on the client thread ("Render
    // thread", "Client thread" before 1.13), so "Started serving on" is
    // logged there, not on the server thread.
    let client = ["[Render thread/INFO]", "[Client thread/INFO]"]
        .iter()
        .any(|t| prefix.contains(t));
    let rest = if prefix.contains("[Server thread/INFO]") {
        MARKERS.iter().find_map(|m| msg.strip_prefix(m))?
    } else if client {
        msg.strip_prefix(MARKERS[0])?
    } else {
        return None;
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() != rest.trim_end().len() {
        return None;
    }
    leading_port(&digits)
}

/// A LAN port seen in a game log (`instance` known) or a local announcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LanSeen {
    pub instance: Option<lc_types::InstanceId>,
    pub port: u16,
}

/// `true` when `ip` belongs to this machine (loopback or a local interface):
/// binding a socket to a foreign address fails.
pub(crate) fn is_local_ip(ip: std::net::IpAddr) -> bool {
    ip.is_loopback() || std::net::UdpSocket::bind((ip, 0)).is_ok()
}
/// Extracts the port from a LAN announcement packet.
pub fn port_from_announcement(packet: &str) -> Option<u16> {
    let start = packet.find("[AD]")? + 4;
    let end = packet[start..].find("[/AD]")? + start;
    let ad = packet[start..end].trim();
    // Some versions put `host:port` here.
    let port = ad.rsplit(':').next().unwrap_or(ad);
    port.parse::<u16>().ok().filter(|p| *p != 0)
}

/// Announcement text the guest broadcasts so the tunnel shows up in the
/// multiplayer screen's LAN list.
pub fn announcement(motd: &str, port: u16) -> String {
    let motd: String = motd
        .chars()
        .filter(|c| !matches!(c, '[' | ']'))
        .take(64)
        .collect();
    format!("[MOTD]{motd}[/MOTD][AD]{port}[/AD]")
}

fn leading_port(s: &str) -> Option<u16> {
    let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<u16>().ok().filter(|p| *p != 0)
}

/// `true` when something accepts TCP on `127.0.0.1:port` (the integrated
/// server listens on all interfaces, so a LAN neighbour's announcement fails
/// this check and is ignored).
pub(crate) async fn is_local_port_open(port: u16) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    matches!(
        tokio::time::timeout(Duration::from_millis(700), TcpStream::connect(addr)).await,
        Ok(Ok(_))
    )
}

fn multicast_socket() -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // The game's own LAN scanner binds the same port.
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, LAN_PORT)).into())?;
    socket.join_multicast_v4(&LAN_GROUP, &Ipv4Addr::UNSPECIFIED)?;
    UdpSocket::from_std(socket.into())
}

/// Listens for LAN announcements and publishes verified local ports into
/// `tx` until `cancel` fires. Errors (port busy without reuse, no multicast
/// route) end the listener quietly: the game log remains a source.
pub(crate) async fn listen_announcements(
    tx: std::sync::Arc<watch::Sender<Option<LanSeen>>>,
    cancel: CancellationToken,
) {
    let socket = match multicast_socket() {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!("lan multicast listener unavailable: {e}");
            return;
        }
    };
    let mut buf = [0u8; 1024];
    loop {
        let res = tokio::select! {
            () = cancel.cancelled() => return,
            r = socket.recv_from(&mut buf) => r,
        };
        let Ok((n, from)) = res else { continue };
        // Only announcements sent by this machine (the host's own game).
        if !is_local_ip(from.ip()) {
            continue;
        }
        let text = String::from_utf8_lossy(&buf[..n]);
        if let Some(port) = port_from_announcement(&text) {
            let seen = LanSeen {
                instance: None,
                port,
            };
            let known = tx.borrow().is_some_and(|s| s.port == port);
            if !known && is_local_port_open(port).await {
                tx.send_replace(Some(seen));
            }
        }
    }
}

/// Broadcasts `[MOTD]..[AD]port` to the local LAN group until cancelled.
pub(crate) async fn announce(motd: String, port: u16, cancel: CancellationToken) {
    let socket = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!("lan announcer unavailable: {e}");
            return;
        }
    };
    // Keep the announcement on this machine.
    let _ = socket.set_multicast_ttl_v4(0);
    let _ = socket.set_multicast_loop_v4(true);
    let packet = announcement(&motd, port);
    let target = SocketAddrV4::new(LAN_GROUP, LAN_PORT);
    loop {
        let _ = socket.send_to(packet.as_bytes(), target).await;
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(ANNOUNCE_EVERY) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_lines() {
        assert_eq!(
            port_from_log_line("[12:00:01] [Server thread/INFO]: Started serving on 51234"),
            Some(51234)
        );
        assert_eq!(
            port_from_log_line("[Server thread/INFO]: Local game hosted on port 41000\r"),
            Some(41000)
        );
        assert_eq!(
            port_from_log_line("[12:00:01] [Server thread/INFO]: Started serving on port"),
            None
        );
        assert_eq!(
            port_from_log_line("[12:00:01] [Server thread/INFO]: Started serving on 99999"),
            None
        );
        assert_eq!(
            port_from_log_line(
                "[22Oct2026 12:00:01.123] [Server thread/INFO] \
                 [net.minecraft.client.server.IntegratedServer/]: Started serving on 40404"
            ),
            Some(40404)
        );
        // Real 1.20+ output: logged by the client thread.
        assert_eq!(
            port_from_log_line("[18:22:41] [Render thread/INFO]: Started serving on 63256"),
            Some(63256)
        );
        assert_eq!(
            port_from_log_line("[18:22:41] [Render thread/INFO]: Local game hosted on port 1"),
            None
        );
        assert_eq!(port_from_log_line("Started serving on 51234"), None);
        assert_eq!(port_from_log_line("Loading 42 mods"), None);
    }

    /// Regression: chat must never move the tunnel target.
    #[test]
    fn chat_lines_are_ignored() {
        for line in [
            "[12:00:01] [Render thread/INFO]: [System] [CHAT] Local game hosted on port 41000",
            "[12:00:02] [Render thread/INFO]: [CHAT] <Mallory> Started serving on 22",
            "[12:00:02] [Server thread/INFO]: [CHAT] <Mallory> Started serving on 22",
            "[12:00:02] [Server thread/INFO]: <Mallory> Started serving on 22",
            "[12:00:02] [Render thread/INFO]: <Mallory> [Server thread/INFO]: Started serving on 22",
            "[12:00:02] [Server thread/INFO]: Started serving on 22 lol",
        ] {
            assert_eq!(port_from_log_line(line), None, "{line}");
        }
    }

    #[test]
    fn local_ips() {
        assert!(is_local_ip("127.0.0.1".parse().unwrap()));
        assert!(!is_local_ip("203.0.113.7".parse().unwrap()));
    }

    #[test]
    fn parses_announcements() {
        assert_eq!(
            port_from_announcement("[MOTD]Steve - World[/MOTD][AD]51234[/AD]"),
            Some(51234)
        );
        assert_eq!(
            port_from_announcement("[MOTD]x[/MOTD][AD]192.168.1.5:4000[/AD]"),
            Some(4000)
        );
        assert_eq!(port_from_announcement("[MOTD]x[/MOTD]"), None);
        assert_eq!(port_from_announcement("[AD]abc[/AD]"), None);
    }

    #[test]
    fn announcement_roundtrip_strips_brackets() {
        let a = announcement("Steve [MineDed]", 40001);
        assert_eq!(a, "[MOTD]Steve MineDed[/MOTD][AD]40001[/AD]");
        assert_eq!(port_from_announcement(&a), Some(40001));
    }

    #[tokio::test]
    async fn local_port_probe() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(is_local_port_open(port).await);
        drop(l);
        assert!(!is_local_port_open(port).await);
    }
}
