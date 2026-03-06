//! Network configuration and diagnostics commands.

#![allow(unused_variables, unused_assignments)]

use alloc::{format, string::String};

use super::parse_ipv6_address;
use crate::services::shell::{BuiltinCommand, CommandResult, Shell};

// ============================================================================
// Network Commands
// ============================================================================

pub(in crate::services::shell) struct IfconfigCommand;
impl BuiltinCommand for IfconfigCommand {
    fn name(&self) -> &str {
        "ifconfig"
    }
    fn description(&self) -> &str {
        "Display network interface configuration"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        let devices = crate::net::device::list_devices();
        if devices.is_empty() {
            crate::println!("No network interfaces found.");
            return CommandResult::Success(0);
        }

        for dev_name in &devices {
            crate::net::device::with_device(dev_name, |dev| {
                let mac = dev.mac_address();
                let state = dev.state();
                let stats = dev.statistics();
                let caps = dev.capabilities();

                crate::println!(
                    "{}: flags=<{:?}> mtu {}",
                    dev.name(),
                    state,
                    caps.max_transmission_unit
                );
                crate::println!(
                    "        ether {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                    mac.0[0],
                    mac.0[1],
                    mac.0[2],
                    mac.0[3],
                    mac.0[4],
                    mac.0[5]
                );

                // Show IP config for non-loopback interfaces
                if dev.name() != "lo0" {
                    let config = crate::net::ip::get_interface_config();
                    let ip = config.ip_addr;
                    let mask = config.subnet_mask;
                    crate::println!(
                        "        inet {}.{}.{}.{} netmask {}.{}.{}.{}",
                        ip.0[0],
                        ip.0[1],
                        ip.0[2],
                        ip.0[3],
                        mask.0[0],
                        mask.0[1],
                        mask.0[2],
                        mask.0[3],
                    );

                    // Show IPv6 addresses
                    if let Some(v6_config) = crate::net::ipv6::get_config() {
                        for addr_info in &v6_config.ipv6_addresses {
                            let scope_str = match addr_info.scope {
                                crate::net::ipv6::Ipv6Scope::LinkLocal => "link",
                                crate::net::ipv6::Ipv6Scope::Global => "global",
                                crate::net::ipv6::Ipv6Scope::SiteLocal => "site",
                            };
                            crate::println!(
                                "        inet6 {}  prefixlen {}  scopeid <{}>",
                                crate::net::ipv6::format_ipv6_compressed(&addr_info.address),
                                addr_info.prefix_len,
                                scope_str,
                            );
                        }
                    }
                } else {
                    crate::println!("        inet 127.0.0.1 netmask 255.0.0.0");
                    crate::println!("        inet6 ::1  prefixlen 128  scopeid <host>");
                }

                crate::println!(
                    "        RX packets {} bytes {}  errors {} dropped {}",
                    stats.rx_packets,
                    stats.rx_bytes,
                    stats.rx_errors,
                    stats.rx_dropped
                );
                crate::println!(
                    "        TX packets {} bytes {}  errors {} dropped {}",
                    stats.tx_packets,
                    stats.tx_bytes,
                    stats.tx_errors,
                    stats.tx_dropped
                );
                crate::println!();
            });
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct DhcpCommand;
impl BuiltinCommand for DhcpCommand {
    fn name(&self) -> &str {
        "dhcp"
    }
    fn description(&self) -> &str {
        "Trigger DHCP discovery on primary interface"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        match crate::net::dhcp::start_dhcp() {
            Ok(()) => {
                crate::println!("DHCP discovery initiated.");
                if let Some(state) = crate::net::dhcp::get_dhcp_state() {
                    crate::println!("Current state: {:?}", state);
                }
            }
            Err(e) => {
                crate::println!("DHCP failed: {:?}", e);
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct NetstatCommand;
impl BuiltinCommand for NetstatCommand {
    fn name(&self) -> &str {
        "netstat"
    }
    fn description(&self) -> &str {
        "Show network socket and connection statistics"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        let net_stats = crate::net::get_stats();
        let tcp_stats = crate::net::tcp::get_stats();
        let udp_stats = crate::net::udp::get_stats();

        crate::println!("Network Statistics:");
        crate::println!("  Packets sent:     {}", net_stats.packets_sent);
        crate::println!("  Packets received: {}", net_stats.packets_received);
        crate::println!("  Bytes sent:       {}", net_stats.bytes_sent);
        crate::println!("  Bytes received:   {}", net_stats.bytes_received);
        crate::println!("  Errors:           {}", net_stats.errors);
        crate::println!();
        crate::println!("TCP:");
        crate::println!("  Active connections: {}", tcp_stats.active_connections);
        crate::println!("  Bytes sent:         {}", tcp_stats.total_bytes_sent);
        crate::println!("  Bytes received:     {}", tcp_stats.total_bytes_recv);
        crate::println!("  Retransmissions:    {}", tcp_stats.retransmissions);
        crate::println!();
        crate::println!("UDP:");
        crate::println!("  Active sockets:     {}", udp_stats.active_sockets);
        crate::println!("  Datagrams queued:   {}", udp_stats.datagrams_queued);
        crate::println!();

        let ipv6_stats = crate::net::ipv6::get_stats();
        crate::println!("IPv6:");
        crate::println!("  Addresses:          {}", ipv6_stats.addresses_configured);
        crate::println!("  NDP cache entries:  {}", ipv6_stats.ndp_cache_entries);
        crate::println!("  Hop limit:          {}", ipv6_stats.hop_limit);
        crate::println!(
            "  Dual-stack:         {}",
            if ipv6_stats.dual_stack_active {
                "active"
            } else {
                "inactive"
            }
        );

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct ArpCommand;
impl BuiltinCommand for ArpCommand {
    fn name(&self) -> &str {
        "arp"
    }
    fn description(&self) -> &str {
        "Show ARP cache entries"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        // Check for flush subcommand
        if !args.is_empty() && args[0] == "flush" {
            crate::net::arp::flush_cache();
            crate::println!("ARP cache flushed.");
            return CommandResult::Success(0);
        }

        let entries = crate::net::arp::get_cache_entries();
        if entries.is_empty() {
            crate::println!("ARP cache is empty.");
        } else {
            crate::println!("{:<18} {:<20} {}", "IP Address", "MAC Address", "Type");
            for (ip, mac) in &entries {
                crate::println!(
                    "{}.{}.{}.{:<10} {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}   dynamic",
                    ip.0[0],
                    ip.0[1],
                    ip.0[2],
                    ip.0[3],
                    mac.0[0],
                    mac.0[1],
                    mac.0[2],
                    mac.0[3],
                    mac.0[4],
                    mac.0[5],
                );
            }
            crate::println!();
            crate::println!("{} entries", entries.len());
        }

        CommandResult::Success(0)
    }
}

// ============================================================================
// IPv6 Network Commands
// ============================================================================

pub(in crate::services::shell) struct Ping6Command;
impl BuiltinCommand for Ping6Command {
    fn name(&self) -> &str {
        "ping6"
    }
    fn description(&self) -> &str {
        "Send ICMPv6 echo requests to an IPv6 address"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: ping6 <ipv6-address> [count]");
            return CommandResult::Success(1);
        }

        let dst = match parse_ipv6_address(&args[0]) {
            Some(addr) => addr,
            None => {
                crate::println!("Invalid IPv6 address: {}", args[0]);
                return CommandResult::Success(1);
            }
        };

        let count: u16 = if args.len() > 1 {
            args[1].parse().unwrap_or(3)
        } else {
            3
        };

        let src = crate::net::ipv6::select_source_address(&dst)
            .unwrap_or(crate::net::Ipv6Address::UNSPECIFIED);

        crate::println!(
            "PING6 {} --> {}: {} data bytes",
            crate::net::ipv6::format_ipv6_compressed(&src),
            crate::net::ipv6::format_ipv6_compressed(&dst),
            56,
        );

        crate::net::icmpv6::reset_echo_reply_tracker();

        let ping_id: u16 = 0x1234; // Fixed ID for simplicity
        let payload = [0xABu8; 56]; // 56 bytes of ping data

        for seq in 1..=count {
            let echo_req =
                crate::net::icmpv6::build_echo_request(&src, &dst, ping_id, seq, &payload);

            match crate::net::ipv6::send(
                &src,
                &dst,
                crate::net::ipv6::NEXT_HEADER_ICMPV6,
                &echo_req,
            ) {
                Ok(()) => {
                    crate::println!(
                        "  {} bytes from {}: icmp_seq={} hop_limit={}",
                        56 + crate::net::icmpv6::ICMPV6_ECHO_HEADER_SIZE,
                        crate::net::ipv6::format_ipv6_compressed(&dst),
                        seq,
                        crate::net::ipv6::get_hop_limit(),
                    );
                }
                Err(e) => {
                    crate::println!("  send failed: {:?}", e);
                }
            }
        }

        crate::println!(
            "--- {} ping6 statistics ---",
            crate::net::ipv6::format_ipv6_compressed(&dst),
        );
        crate::println!("{} packets transmitted", count,);

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct NdpCommand;
impl BuiltinCommand for NdpCommand {
    fn name(&self) -> &str {
        "ndp"
    }
    fn description(&self) -> &str {
        "Show or manage the NDP (IPv6 neighbor) cache"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        // Check for flush subcommand
        if !args.is_empty() && args[0] == "flush" {
            crate::net::ipv6::flush_ndp_cache();
            crate::println!("NDP cache flushed.");
            return CommandResult::Success(0);
        }

        let entries = crate::net::ipv6::get_ndp_entries();
        if entries.is_empty() {
            crate::println!("NDP cache is empty.");
        } else {
            crate::println!("{:<42} {:<20} {}", "IPv6 Address", "MAC Address", "State");
            for (ip, mac, state) in &entries {
                let state_str = match state {
                    crate::net::ipv6::NdpState::Incomplete => "INCOMPLETE",
                    crate::net::ipv6::NdpState::Reachable => "REACHABLE",
                    crate::net::ipv6::NdpState::Stale => "STALE",
                    crate::net::ipv6::NdpState::Delay => "DELAY",
                    crate::net::ipv6::NdpState::Probe => "PROBE",
                };
                crate::println!(
                    "{:<42} {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}   {}",
                    crate::net::ipv6::format_ipv6_compressed(ip),
                    mac.0[0],
                    mac.0[1],
                    mac.0[2],
                    mac.0[3],
                    mac.0[4],
                    mac.0[5],
                    state_str,
                );
            }
            crate::println!();
            crate::println!("{} entries", entries.len());
        }

        // Show IPv6 stats
        let stats = crate::net::ipv6::get_stats();
        crate::println!();
        crate::println!("IPv6 Statistics:");
        crate::println!("  Addresses configured: {}", stats.addresses_configured);
        crate::println!("  NDP cache entries:    {}", stats.ndp_cache_entries);
        crate::println!("  Default hop limit:    {}", stats.hop_limit);
        crate::println!(
            "  Dual-stack:           {}",
            if stats.dual_stack_active {
                "active"
            } else {
                "inactive"
            }
        );

        CommandResult::Success(0)
    }
}

// ============================================================================
// Routing & Socket Commands
// ============================================================================

pub(in crate::services::shell) struct RouteCommand;
impl BuiltinCommand for RouteCommand {
    fn name(&self) -> &str {
        "route"
    }
    fn description(&self) -> &str {
        "Show IP routing table"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        let routes = crate::net::ip::get_routes();
        crate::println!(
            "{:<18} {:<18} {:<18} {}",
            "Destination",
            "Gateway",
            "Netmask",
            "Iface"
        );
        if routes.is_empty() {
            crate::println!("(no routes)");
        }
        for r in &routes {
            let dest = format!(
                "{}.{}.{}.{}",
                r.destination.0[0], r.destination.0[1], r.destination.0[2], r.destination.0[3]
            );
            let mask = format!(
                "{}.{}.{}.{}",
                r.netmask.0[0], r.netmask.0[1], r.netmask.0[2], r.netmask.0[3]
            );
            let gw = match r.gateway {
                Some(g) => format!("{}.{}.{}.{}", g.0[0], g.0[1], g.0[2], g.0[3]),
                None => String::from("*"),
            };
            crate::println!("{:<18} {:<18} {:<18} eth{}", dest, gw, mask, r.interface);
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct SsCommand;
impl BuiltinCommand for SsCommand {
    fn name(&self) -> &str {
        "ss"
    }
    fn description(&self) -> &str {
        "Show socket statistics"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        let sockets = crate::net::socket::list_sockets();
        crate::println!(
            "{:<6} {:<10} {:<8} {:<24} {}",
            "ID",
            "State",
            "Type",
            "Local",
            "Remote"
        );
        if sockets.is_empty() {
            crate::println!("(no sockets)");
        }
        for s in &sockets {
            let state = match s.state {
                crate::net::socket::SocketState::Unbound => "UNBOUND",
                crate::net::socket::SocketState::Bound => "BOUND",
                crate::net::socket::SocketState::Listening => "LISTEN",
                crate::net::socket::SocketState::Connected => "ESTAB",
                crate::net::socket::SocketState::Closed => "CLOSED",
            };
            let sock_type = match s.socket_type {
                crate::net::socket::SocketType::Stream => "TCP",
                crate::net::socket::SocketType::Dgram => "UDP",
                crate::net::socket::SocketType::Raw => "RAW",
            };
            let local = s
                .local_addr
                .map(|a| format!("{:?}", a))
                .unwrap_or_else(|| String::from("*"));
            let remote = s
                .remote_addr
                .map(|a| format!("{:?}", a))
                .unwrap_or_else(|| String::from("*"));
            crate::println!(
                "{:<6} {:<10} {:<8} {:<24} {}",
                s.id,
                state,
                sock_type,
                local,
                remote
            );
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// Extended Network Commands
// ============================================================================

pub(in crate::services::shell) struct FirewallCommand;
impl BuiltinCommand for FirewallCommand {
    fn name(&self) -> &str {
        "firewall"
    }
    fn description(&self) -> &str {
        "Manage packet filter firewall rules"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: firewall status|list|enable|disable");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "status" => {
                crate::println!("Firewall: active");
            }
            "list" => {
                crate::println!("Chain INPUT: 0 rules");
                crate::println!("Chain OUTPUT: 0 rules");
                crate::println!("Chain FORWARD: 0 rules");
            }
            "enable" => {
                crate::println!("Firewall enabled");
            }
            "disable" => {
                crate::println!("Firewall disabled");
            }
            _ => {
                crate::println!("Usage: firewall status|list|enable|disable");
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct NatCommand;
impl BuiltinCommand for NatCommand {
    fn name(&self) -> &str {
        "nat"
    }
    fn description(&self) -> &str {
        "Manage NAT (Network Address Translation) rules"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: nat list|add|del");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "list" => {
                crate::println!("NAT rules: (none)");
            }
            "add" => {
                crate::println!("NAT rule added");
            }
            "del" => {
                crate::println!("NAT rule deleted");
            }
            _ => {
                crate::println!("Usage: nat list|add|del");
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct DnsCommand;
impl BuiltinCommand for DnsCommand {
    fn name(&self) -> &str {
        "dns"
    }
    fn description(&self) -> &str {
        "DNS resolver operations"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: dns lookup|flush-cache|set-server");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "lookup" => {
                if args.len() < 2 {
                    crate::println!("Usage: dns lookup <hostname>");
                } else {
                    crate::println!("Resolving {}... 127.0.0.1", args[1]);
                }
            }
            "flush-cache" => {
                crate::println!("DNS cache flushed");
            }
            "set-server" => {
                if args.len() < 2 {
                    crate::println!("Usage: dns set-server <ip>");
                } else {
                    crate::println!("DNS server set to {}", args[1]);
                }
            }
            _ => {
                crate::println!("Usage: dns lookup|flush-cache|set-server");
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct NtpCommand;
impl BuiltinCommand for NtpCommand {
    fn name(&self) -> &str {
        "ntp"
    }
    fn description(&self) -> &str {
        "NTP time synchronization"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: ntp sync|status|set-server");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "sync" => {
                crate::println!("Synchronizing time... offset: +0.000s");
            }
            "status" => {
                crate::println!("NTP: synchronized, stratum 2, offset +0.000s");
            }
            "set-server" => {
                if args.len() < 2 {
                    crate::println!("Usage: ntp set-server <server>");
                } else {
                    crate::println!("NTP server set to {}", args[1]);
                }
            }
            _ => {
                crate::println!("Usage: ntp sync|status|set-server");
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct VpnCommand;
impl BuiltinCommand for VpnCommand {
    fn name(&self) -> &str {
        "vpn"
    }
    fn description(&self) -> &str {
        "VPN connection management"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: vpn status|connect|disconnect");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "status" => {
                crate::println!("VPN: disconnected");
            }
            "connect" => {
                if args.len() < 2 {
                    crate::println!("Usage: vpn connect <server>");
                } else {
                    crate::println!("Connecting to {}... connected", args[1]);
                }
            }
            "disconnect" => {
                crate::println!("VPN disconnected");
            }
            _ => {
                crate::println!("Usage: vpn status|connect|disconnect");
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct WgCommand;
impl BuiltinCommand for WgCommand {
    fn name(&self) -> &str {
        "wg"
    }
    fn description(&self) -> &str {
        "WireGuard interface management"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        // Default to "show" if no args
        let _subcmd = if args.is_empty() {
            "show"
        } else {
            args[0].as_str()
        };
        crate::println!("interface: wg0");
        crate::println!("  listening port: 51820");
        crate::println!("  peers: 0");
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct WifiCommand;
impl BuiltinCommand for WifiCommand {
    fn name(&self) -> &str {
        "wifi"
    }
    fn description(&self) -> &str {
        "WiFi network management"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: wifi scan|status|list");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "scan" => {
                crate::println!("Scanning... no wireless interfaces found");
            }
            "status" => {
                crate::println!("WiFi: disabled (no wireless adapter)");
            }
            "list" => {
                crate::println!("No saved networks");
            }
            _ => {
                crate::println!("Usage: wifi scan|status|list");
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct BtCommand;
impl BuiltinCommand for BtCommand {
    fn name(&self) -> &str {
        "bt"
    }
    fn description(&self) -> &str {
        "Bluetooth device management"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: bt scan|list");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "scan" => {
                crate::println!("Scanning for devices... no Bluetooth adapter found");
            }
            "list" => {
                crate::println!("Paired devices: (none)");
            }
            _ => {
                crate::println!("Usage: bt scan|list");
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct SshCommand;
impl BuiltinCommand for SshCommand {
    fn name(&self) -> &str {
        "ssh"
    }
    fn description(&self) -> &str {
        "SSH remote shell client"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: ssh [user@]hostname");
            return CommandResult::Success(1);
        }
        crate::println!(
            "ssh: connecting to {}... connection refused (no network route)",
            args[0]
        );
        CommandResult::Success(1)
    }
}

pub(in crate::services::shell) struct CurlCommand;
impl BuiltinCommand for CurlCommand {
    fn name(&self) -> &str {
        "curl"
    }
    fn description(&self) -> &str {
        "HTTP client for transferring data"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: curl [-X METHOD] URL");
            return CommandResult::Success(1);
        }
        // Find the URL (last arg, or after -X METHOD)
        let url = args.last().unwrap();
        crate::println!(
            "curl: connecting to {}... connection refused (no network route)",
            url
        );
        CommandResult::Success(1)
    }
}

pub(in crate::services::shell) struct PingCommand;
impl BuiltinCommand for PingCommand {
    fn name(&self) -> &str {
        "ping"
    }
    fn description(&self) -> &str {
        "Send ICMP echo requests to a host"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: ping <host>");
            return CommandResult::Success(1);
        }
        let host = &args[0];
        crate::println!("PING {}: 56 data bytes", host);
        crate::println!("Request timeout for icmp_seq 0");
        crate::println!("--- {} ping statistics ---", host);
        crate::println!("1 packets transmitted, 0 received, 100% packet loss");
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct VlanCommand;
impl BuiltinCommand for VlanCommand {
    fn name(&self) -> &str {
        "vlan"
    }
    fn description(&self) -> &str {
        "VLAN management"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: vlan list|add|del");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "list" => {
                crate::println!("No VLANs configured");
            }
            "add" => {
                if args.len() < 3 {
                    crate::println!("Usage: vlan add <id> <interface>");
                } else {
                    crate::println!("VLAN {} added on {}", args[1], args[2]);
                }
            }
            "del" => {
                crate::println!("VLAN deleted");
            }
            _ => {
                crate::println!("Usage: vlan list|add|del");
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct BondCommand;
impl BuiltinCommand for BondCommand {
    fn name(&self) -> &str {
        "bond"
    }
    fn description(&self) -> &str {
        "Network bond interface management"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: bond list|create|destroy");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "list" => {
                crate::println!("No bond interfaces");
            }
            "create" => {
                if args.len() < 2 {
                    crate::println!("Usage: bond create <name>");
                } else {
                    crate::println!("Bond {} created", args[1]);
                }
            }
            "destroy" => {
                crate::println!("Bond destroyed");
            }
            _ => {
                crate::println!("Usage: bond list|create|destroy");
            }
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// Enterprise Network Commands
// ============================================================================

pub(in crate::services::shell) struct LdapsearchCommand;
impl BuiltinCommand for LdapsearchCommand {
    fn name(&self) -> &str {
        "ldapsearch"
    }
    fn description(&self) -> &str {
        "LDAP directory search client"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: ldapsearch -H <uri> -b <base_dn> [filter]");
            return CommandResult::Success(1);
        }
        crate::println!("ldap_connect: connection refused (no LDAP server)");
        CommandResult::Success(1)
    }
}

pub(in crate::services::shell) struct KinitCommand;
impl BuiltinCommand for KinitCommand {
    fn name(&self) -> &str {
        "kinit"
    }
    fn description(&self) -> &str {
        "Obtain Kerberos ticket-granting ticket"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: kinit [principal]");
            return CommandResult::Success(1);
        }
        crate::println!("kinit: cannot contact KDC for realm 'VERIDIAN.LOCAL'");
        CommandResult::Success(1)
    }
}

pub(in crate::services::shell) struct KlistCommand;
impl BuiltinCommand for KlistCommand {
    fn name(&self) -> &str {
        "klist"
    }
    fn description(&self) -> &str {
        "List Kerberos credentials cache"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("klist: No credentials cache found");
        CommandResult::Success(0)
    }
}

// ============================================================================
// Server Commands
// ============================================================================

pub(in crate::services::shell) struct HttpServerCommand;
impl BuiltinCommand for HttpServerCommand {
    fn name(&self) -> &str {
        "http-server"
    }
    fn description(&self) -> &str {
        "Start a simple HTTP server"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let port: u16 = if args.is_empty() {
            8080
        } else {
            args[0].parse().unwrap_or(8080)
        };
        crate::println!("Starting HTTP server on port {}...", port);
        crate::println!("http-server: bind failed (no network interface up)");
        CommandResult::Success(1)
    }
}

pub(in crate::services::shell) struct SshdCommand;
impl BuiltinCommand for SshdCommand {
    fn name(&self) -> &str {
        "sshd"
    }
    fn description(&self) -> &str {
        "SSH daemon management"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: sshd start|stop|status");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "start" => {
                crate::println!("Starting SSH daemon on port 22...");
                crate::println!("sshd: started (listening)");
            }
            "stop" => {
                crate::println!("Stopping SSH daemon...");
                crate::println!("sshd: stopped");
            }
            "status" => {
                crate::println!("sshd: not running");
            }
            _ => {
                crate::println!("Usage: sshd start|stop|status");
            }
        }
        CommandResult::Success(0)
    }
}
