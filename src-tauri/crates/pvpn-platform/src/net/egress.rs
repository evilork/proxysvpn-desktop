// src-tauri/crates/pvpn-platform/src/net/egress.rs
//
// A TCP connection that leaves through the physical interface, whatever the
// routing table says.
//
// Why it exists. While the tunnel is up, `0.0.0.0/1` and `128.0.0.0/1` point
// into it, and only the node the engines are connected to has a host route
// around it. An ordinary socket to any OTHER node therefore goes into our own
// tunnel, where tun2socks completes the TCP handshake locally before it has
// sent anything anywhere: the Countries list showed 1-5 ms for every location
// with the VPN on and 230-400 ms with it off (Debian and Windows 11 VMs,
// 02.10.2026). Pinning the socket to the physical interface measures what the
// list means — the way from this machine to the node — on all three systems,
// without touching a single route.
//
// The pin is the one xray already uses for its own outbounds
// (`sockopt.interface`, xray_manager.rs), so it is known to work where the
// tunnel works:
//   macOS    IP_BOUND_IF / IPV6_BOUND_IF with the interface index — scoped
//            routing, no privilege needed;
//   Linux    SO_BINDTODEVICE with the interface name — open to unprivileged
//            processes since Linux 5.7, which is how the unprivileged xray of
//            the Linux build binds today;
//   Windows  IP_UNICAST_IF / IPV6_UNICAST_IF with the index of the adapter
//            whose alias the physical route names.
//
// A pin that cannot be set is reported apart from a connect that fails: the
// first means "this machine cannot measure around the tunnel right now" and
// nothing was sent, the second is an answer about the node.

use std::io;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use socket2::{Domain, Protocol, SockAddr, Socket, Type};

/// Why a pinned connection did not happen.
#[derive(Debug)]
pub enum PinnedConnectError {
    /// The socket could not be pinned to the interface (no such interface, a
    /// kernel that refuses the option). Nothing left the machine.
    Pin(io::Error),
    /// Pinned, then the connect itself failed: refused, unreachable, timed
    /// out. An answer about the node, not about this machine.
    Connect(io::Error),
}

impl std::fmt::Display for PinnedConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pin(e) => write!(f, "pin to the physical interface: {e}"),
            Self::Connect(e) => write!(f, "connect: {e}"),
        }
    }
}

impl std::error::Error for PinnedConnectError {}

/// Connect to `addr` through the interface called `if_name` (`en0`, `eth0`,
/// the Windows adapter alias "Ethernet"), bounded by `timeout`.
pub fn connect_pinned(
    addr: SocketAddr,
    if_name: &str,
    timeout: Duration,
) -> Result<TcpStream, PinnedConnectError> {
    if if_name.trim().is_empty() {
        // An empty name would mean "do not bind" to two of the three kernels,
        // i.e. silently back into the tunnel.
        return Err(PinnedConnectError::Pin(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty interface name",
        )));
    }
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))
        .map_err(PinnedConnectError::Pin)?;
    pin(&socket, addr, if_name).map_err(PinnedConnectError::Pin)?;
    socket
        .connect_timeout(&SockAddr::from(addr), timeout)
        .map_err(PinnedConnectError::Connect)?;
    Ok(socket.into())
}

#[cfg(target_os = "macos")]
fn pin(socket: &Socket, addr: SocketAddr, if_name: &str) -> io::Result<()> {
    let name = std::ffi::CString::new(if_name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "interface name holds a NUL"))?;
    // SAFETY: `name` is a valid NUL-terminated string that outlives the call.
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    let index = std::num::NonZeroU32::new(index)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no interface {if_name}")))?;
    match addr {
        SocketAddr::V4(_) => socket.bind_device_by_index_v4(Some(index)),
        SocketAddr::V6(_) => socket.bind_device_by_index_v6(Some(index)),
    }
}

#[cfg(target_os = "linux")]
fn pin(socket: &Socket, _addr: SocketAddr, if_name: &str) -> io::Result<()> {
    // SO_BINDTODEVICE serves both families.
    socket.bind_device(Some(if_name.as_bytes()))
}

#[cfg(target_os = "windows")]
fn pin(socket: &Socket, addr: SocketAddr, if_name: &str) -> io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::NO_ERROR;
    use windows::Win32::NetworkManagement::IpHelper::{
        ConvertInterfaceAliasToLuid, ConvertInterfaceLuidToIndex,
    };
    use windows::Win32::NetworkManagement::Ndis::NET_LUID_LH;
    use windows::Win32::Networking::WinSock::{
        setsockopt, IPPROTO_IP, IPPROTO_IPV6, IPV6_UNICAST_IF, IP_UNICAST_IF, SOCKET,
    };

    let alias: Vec<u16> = if_name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut luid = NET_LUID_LH::default();
    // SAFETY: `alias` is NUL-terminated UTF-16 that outlives the call, and
    // `luid` is a live out-parameter.
    if unsafe { ConvertInterfaceAliasToLuid(PCWSTR(alias.as_ptr()), &mut luid) } != NO_ERROR {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no adapter with the alias {if_name}"),
        ));
    }
    let mut index = 0u32;
    // SAFETY: `luid` was filled in above; `index` is a live out-parameter.
    if unsafe { ConvertInterfaceLuidToIndex(&luid, &mut index) } != NO_ERROR || index == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no index for the adapter {if_name}"),
        ));
    }
    // IPv4 takes the index in network byte order, IPv6 in host order — the
    // documented asymmetry of IP_UNICAST_IF and IPV6_UNICAST_IF.
    let (level, option, value) = match addr {
        SocketAddr::V4(_) => (IPPROTO_IP.0, IP_UNICAST_IF, index.to_be()),
        SocketAddr::V6(_) => (IPPROTO_IPV6.0, IPV6_UNICAST_IF, index),
    };
    let raw = SOCKET(socket.as_raw_socket() as usize);
    // SAFETY: `raw` is the live socket owned by `socket`, and the value is a
    // four-byte DWORD as both options require.
    if unsafe { setsockopt(raw, level, option, Some(&value.to_ne_bytes())) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_interface_name_is_refused_before_anything_is_sent() {
        let addr: SocketAddr = "203.0.113.9:443".parse().expect("addr");
        match connect_pinned(addr, "  ", Duration::from_millis(50)) {
            Err(PinnedConnectError::Pin(_)) => {}
            other => panic!("expected a pin error, got {other:?}"),
        }
    }

    #[test]
    fn an_interface_that_does_not_exist_is_a_pin_error_not_an_answer_about_the_node() {
        // TEST-NET-3: had the connect been attempted unpinned, the test would
        // sit out the whole timeout instead of failing at once.
        let addr: SocketAddr = "203.0.113.9:443".parse().expect("addr");
        match connect_pinned(addr, "pvpn-no-such-if0", Duration::from_secs(2)) {
            Err(PinnedConnectError::Pin(_)) => {}
            other => panic!("expected a pin error, got {other:?}"),
        }
    }

    /// The loopback interface stands in for the physical one: a socket pinned
    /// to it reaches a listener on 127.0.0.1, so the pin itself is set the
    /// way the kernel accepts. Unix only: the Windows loopback alias differs
    /// between builds, and Windows is cross-checked from a Mac.
    #[cfg(unix)]
    #[test]
    fn a_socket_pinned_to_loopback_reaches_a_loopback_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let lo = if cfg!(target_os = "macos") { "lo0" } else { "lo" };
        match connect_pinned(addr, lo, Duration::from_secs(2)) {
            Ok(stream) => assert_eq!(stream.peer_addr().expect("peer"), addr),
            // A Linux kernel older than 5.7 refuses SO_BINDTODEVICE to
            // unprivileged processes; the tunnel's own xray would fail there
            // the same way, so that is not this code's failure.
            Err(PinnedConnectError::Pin(e)) if e.kind() == io::ErrorKind::PermissionDenied => {}
            Err(e) => panic!("pinned loopback connect failed: {e}"),
        }
    }
}
