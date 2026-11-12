//! 本机 IP（v0.2 U2，设置页「本机 IP」）。每次打开设置页、每次展开时重新读取，
//! 换 Wi-Fi、连 VPN 后不用重启程序。

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// 一块网卡上的 IPv4 地址。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalIp {
    pub nic: String,
    pub ip: String,
}

/// 可供联系人填写的本机 IP：IPv4、非回环，并过滤 169.254.0.0/16（Windows 上断开的网卡
/// 常显示这个自动分配地址，填了也连不上）。物理网卡在前（默认路由所在的排第一），
/// 虚拟网卡排最后；收起时的摘要取第一行。
pub fn local_ips() -> Vec<LocalIp> {
    let all = local_ip_address::list_afinet_netifas().unwrap_or_default();
    arrange(all, primary_ipv4())
}

/// 默认路由使用的源地址。UDP 的 connect 只选路由、不发送任何数据包；
/// 目标取文档保留地址（RFC 5737），没有默认路由时返回 `None`。
fn primary_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

/// 虚拟网卡：Hyper-V / WSL、Docker、网桥、VPN / 代理隧道、虚拟机软件等。
/// 198.18.0.0/15 是测试保留网段（RFC 2544），实际上只被 Clash / Mihomo 等代理的 TUN 网卡使用。
fn is_virtual(nic: &str, ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    if a == 198 && (b & 0xFE) == 18 {
        return true;
    }
    const PREFIXES: [&str; 17] = [
        "vethernet",
        "docker",
        "br-",
        "veth",
        "virbr",
        "vmnet",
        "vboxnet",
        "lxcbr",
        "lxdbr",
        "podman",
        "cni",
        "flannel",
        "tun",
        "tap",
        "wg",
        "zt",
        "tailscale",
    ];
    const CONTAINS: [&str; 8] = [
        "virtualbox",
        "vmware",
        "hyper-v",
        "mihomo",
        "clash",
        "sing-box",
        "singbox",
        "wintun",
    ];
    let lower = nic.to_lowercase();
    PREFIXES.iter().any(|p| lower.starts_with(p)) || CONTAINS.iter().any(|c| lower.contains(c))
}

fn arrange(all: Vec<(String, IpAddr)>, primary: Option<Ipv4Addr>) -> Vec<LocalIp> {
    let mut list: Vec<(String, Ipv4Addr)> = all
        .into_iter()
        .filter_map(|(nic, ip)| match ip {
            IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() => {
                Some((nic, ip))
            }
            _ => None,
        })
        .collect();
    // 稳定排序：物理网卡在前、虚拟网卡在后，各自默认路由所在的排第一，其余保持系统顺序。
    // 开着代理 TUN 时默认路由指向 TUN 网卡，它对局域网联系人没有用，所以虚拟优先于默认路由判断。
    list.sort_by_key(|(nic, ip)| (is_virtual(nic, *ip), Some(*ip) != primary));
    list.dedup_by(|a, b| a == b);
    list.into_iter()
        .map(|(nic, ip)| LocalIp {
            nic,
            ip: ip.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(s: &str) -> IpAddr {
        IpAddr::V4(s.parse().unwrap())
    }

    #[test]
    fn filters_and_orders() {
        let all = vec![
            ("lo".to_string(), v4("127.0.0.1")),
            ("docker0".to_string(), v4("172.17.0.1")),
            ("以太网 2".to_string(), v4("169.254.10.3")),
            (
                "vEthernet (WSL (Hyper-V firewall))".to_string(),
                v4("172.29.48.1"),
            ),
            ("WLAN".to_string(), v4("10.8.0.3")),
            ("以太网".to_string(), v4("192.168.1.10")),
            ("WLAN".to_string(), "fe80::1".parse().unwrap()),
        ];
        let ips = arrange(all.clone(), Some("192.168.1.10".parse().unwrap()));
        let pairs: Vec<(&str, &str)> = ips
            .iter()
            .map(|i| (i.nic.as_str(), i.ip.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("以太网", "192.168.1.10"),
                ("WLAN", "10.8.0.3"),
                ("docker0", "172.17.0.1"),
                ("vEthernet (WSL (Hyper-V firewall))", "172.29.48.1"),
            ]
        );
        // 没有默认路由：物理网卡在前，系统顺序不变。
        let ips = arrange(all.clone(), None);
        assert_eq!(ips[0].ip, "10.8.0.3");
        assert_eq!(ips[1].ip, "192.168.1.10");
        // 默认路由走代理 TUN（Mihomo fake-ip）时，TUN 网卡仍排在物理网卡之后。
        let mut with_tun = all;
        with_tun.insert(0, ("Mihomo".to_string(), v4("198.18.0.1")));
        let ips = arrange(with_tun, Some("198.18.0.1".parse().unwrap()));
        assert_eq!(ips[0].ip, "10.8.0.3");
        assert_eq!(ips[2].nic, "Mihomo");
        assert!(arrange(vec![("lo".into(), v4("127.0.0.1"))], None).is_empty());
    }

    #[test]
    fn virtual_nics() {
        for nic in [
            "docker0",
            "br-5c1e8f2a9d3b",
            "vEthernet (Default Switch)",
            "virbr0",
            "tailscale0",
            "Mihomo",
        ] {
            assert!(is_virtual(nic, Ipv4Addr::new(10, 0, 0, 1)), "{nic}");
        }
        for nic in ["enp0s31f6", "wlp0s20f3", "以太网", "WLAN", "eth0", "Wi-Fi"] {
            assert!(!is_virtual(nic, Ipv4Addr::new(10, 0, 0, 1)), "{nic}");
        }
        assert!(is_virtual("utun", Ipv4Addr::new(198, 19, 0, 1)));
        assert!(!is_virtual("enp7s0", Ipv4Addr::new(198, 20, 0, 1)));
    }
}
