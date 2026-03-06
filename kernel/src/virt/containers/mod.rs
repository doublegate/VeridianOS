//! Enhanced container runtime with OCI specification support, cgroup
//! controllers, overlay filesystem, veth networking, and seccomp BPF filtering.
//!
//! This module implements 7 container enhancement sprints:
//! 1. OCI Runtime Specification (config.json parsing, lifecycle, hooks,
//!    pivot_root)
//! 2. Container Image Format (layers, overlay composition, manifest, SHA-256
//!    IDs)
//! 3. Cgroup Memory Controller (limits, usage tracking, OOM, hierarchical
//!    accounting)
//! 4. Cgroup CPU Controller (shares, quota/period, throttling, burst,
//!    hierarchy)
//! 5. Overlay Filesystem (lower/upper layers, copy-up, whiteout, directory
//!    merge)
//! 6. Veth Networking (virtual pairs, bridge, NAT masquerade, ARP proxy, MTU)
//! 7. Seccomp BPF (filter instructions, syscall filtering, arg inspection,
//!    inheritance)

mod cgroups;
mod image;
mod networking;
mod oci;
mod overlay;
mod seccomp;

// Re-export everything so external consumers see no change.
pub use self::{cgroups::*, image::*, networking::*, oci::*, overlay::*, seccomp::*};

// ---------------------------------------------------------------------------
// Shared helpers used across submodules
// ---------------------------------------------------------------------------

/// Parse a u32 from a decimal string.
pub(crate) fn parse_u32(s: &str) -> Option<u32> {
    let mut result: u32 = 0;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            result = result.checked_mul(10)?;
            result = result.checked_add((b - b'0') as u32)?;
        } else {
            return None;
        }
    }
    Some(result)
}

/// Parse a u64 from a decimal string.
pub(crate) fn parse_u64(s: &str) -> Option<u64> {
    let mut result: u64 = 0;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            result = result.checked_mul(10)?;
            result = result.checked_add((b - b'0') as u64)?;
        } else {
            return None;
        }
    }
    Some(result)
}

/// Minimal SHA-256 implementation (same algorithm as crypto::hash::sha256
/// but self-contained to avoid circular dependencies).
pub(crate) fn simple_sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let original_len_bits = (data.len() as u64).saturating_mul(8);

    // Pad message: append 0x80, zeros, then 64-bit big-endian length
    let padded_len = (data.len() + 9).div_ceil(64) * 64;
    // Use a stack buffer for small inputs, otherwise heap
    let mut padded = [0u8; 128]; // enough for up to 119 bytes of input
    let use_stack = padded_len <= 128;

    #[cfg(feature = "alloc")]
    let mut heap_padded: alloc::vec::Vec<u8>;
    #[cfg(not(feature = "alloc"))]
    let heap_padded: [u8; 0] = [];

    let buf: &mut [u8] = if use_stack {
        padded[..data.len()].copy_from_slice(data);
        padded[data.len()] = 0x80;
        let len_offset = padded_len - 8;
        padded[len_offset..len_offset + 8].copy_from_slice(&original_len_bits.to_be_bytes());
        &mut padded[..padded_len]
    } else {
        #[cfg(feature = "alloc")]
        {
            heap_padded = alloc::vec![0u8; padded_len];
            heap_padded[..data.len()].copy_from_slice(data);
            heap_padded[data.len()] = 0x80;
            let len_offset = padded_len - 8;
            heap_padded[len_offset..len_offset + 8]
                .copy_from_slice(&original_len_bits.to_be_bytes());
            &mut heap_padded[..]
        }
        #[cfg(not(feature = "alloc"))]
        {
            // Without alloc, we cannot handle inputs > 119 bytes.
            // Return zeros as a safe fallback.
            return [0u8; 32];
        }
    };

    // Process 64-byte blocks
    let mut w = [0u32; 64];
    let mut block_offset = 0;
    while block_offset < buf.len() {
        let block = &buf[block_offset..block_offset + 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);

        block_offset += 64;
    }

    let mut out = [0u8; 32];
    for (i, &val) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&val.to_be_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // --- OCI Runtime Spec tests ---

    #[test]
    fn test_oci_lifecycle_state_display() {
        assert_eq!(
            alloc::format!("{}", OciLifecycleState::Creating),
            "creating"
        );
        assert_eq!(alloc::format!("{}", OciLifecycleState::Created), "created");
        assert_eq!(alloc::format!("{}", OciLifecycleState::Running), "running");
        assert_eq!(alloc::format!("{}", OciLifecycleState::Stopped), "stopped");
    }

    #[test]
    fn test_oci_namespace_kind_parse() {
        assert_eq!(
            OciNamespaceKind::from_str_kind("pid"),
            Some(OciNamespaceKind::Pid)
        );
        assert_eq!(
            OciNamespaceKind::from_str_kind("network"),
            Some(OciNamespaceKind::Network)
        );
        assert_eq!(OciNamespaceKind::from_str_kind("invalid"), None);
    }

    #[test]
    fn test_oci_config_parse_basic() {
        let input = concat!(
            "oci_version=1.0.2\n",
            "root_path=/rootfs\n",
            "root_readonly=true\n",
            "hostname=mycontainer\n",
            "process_cwd=/app\n",
            "process_uid=1000\n",
            "process_gid=1000\n",
            "process_terminal=true\n",
            "process_arg=/bin/sh\n",
            "process_arg=-c\n",
            "process_env=PATH=/usr/bin\n",
            "namespace=pid\n",
            "namespace=network:/proc/123/ns/net\n",
            "cgroups_path=/sys/fs/cgroup/mycontainer\n",
            "memory_limit=67108864\n",
            "cpu_shares=512\n",
            "cpu_quota=50000\n",
            "cpu_period=100000\n",
            "hook_prestart=/usr/bin/hook:5\n",
            "mount=/proc:proc:proc:nosuid,noexec\n",
        );
        let config = OciConfig::parse(input).unwrap();
        assert_eq!(config.oci_version, "1.0.2");
        assert_eq!(config.root.path, "/rootfs");
        assert!(config.root.readonly);
        assert_eq!(config.hostname, "mycontainer");
        assert_eq!(config.process.cwd, "/app");
        assert_eq!(config.process.uid, 1000);
        assert_eq!(config.process.gid, 1000);
        assert!(config.process.terminal);
        assert_eq!(config.process.args.len(), 2);
        assert_eq!(config.process.args[0], "/bin/sh");
        assert_eq!(config.process.env.len(), 1);
        assert_eq!(config.linux.namespaces.len(), 2);
        assert_eq!(config.linux.namespaces[0].kind, OciNamespaceKind::Pid);
        assert!(config.linux.namespaces[0].path.is_none());
        assert_eq!(config.linux.namespaces[1].kind, OciNamespaceKind::Network);
        assert!(config.linux.namespaces[1].path.is_some());
        assert_eq!(config.linux.memory_limit, 67108864);
        assert_eq!(config.linux.cpu_shares, 512);
        assert_eq!(config.linux.cpu_quota, 50000);
        assert_eq!(config.hooks.prestart.len(), 1);
        assert_eq!(config.hooks.prestart[0].timeout_secs, 5);
        assert_eq!(config.mounts.len(), 1);
        assert_eq!(config.mounts[0].options.len(), 2);
    }

    #[test]
    fn test_oci_config_validate_empty_args() {
        let input = "root_path=/rootfs\n";
        let config = OciConfig::parse(input).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_oci_container_lifecycle() {
        let input = "root_path=/rootfs\nprocess_arg=/bin/sh\nprocess_cwd=/\n";
        let config = OciConfig::parse(input).unwrap();
        let mut container = OciContainer::new("test1", "/bundles/test1", config).unwrap();
        assert_eq!(container.state, OciLifecycleState::Creating);

        container.mark_created().unwrap();
        assert_eq!(container.state, OciLifecycleState::Created);

        container.start(42).unwrap();
        assert_eq!(container.state, OciLifecycleState::Running);
        assert_eq!(container.pid, 42);

        container.stop().unwrap();
        assert_eq!(container.state, OciLifecycleState::Stopped);
    }

    #[test]
    fn test_oci_container_invalid_transition() {
        let input = "root_path=/rootfs\nprocess_arg=/bin/sh\nprocess_cwd=/\n";
        let config = OciConfig::parse(input).unwrap();
        let mut container = OciContainer::new("test1", "/bundles/test1", config).unwrap();
        // Cannot start from Creating (must be Created first)
        assert!(container.start(1).is_err());
    }

    #[test]
    fn test_oci_container_pivot_root() {
        let input = "root_path=/rootfs\nprocess_arg=/bin/sh\nprocess_cwd=/\n";
        let config = OciConfig::parse(input).unwrap();
        let container = OciContainer::new("test1", "/bundles/test1", config).unwrap();
        let (old, new) = container.pivot_root().unwrap();
        assert_eq!(old, "/.pivot_root");
        assert_eq!(new, "/rootfs");
    }

    // --- Container Image Format tests ---

    #[test]
    fn test_layer_digest_compute() {
        let digest = LayerDigest::compute(b"hello");
        // SHA-256("hello") =
        // 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        assert_eq!(digest.bytes[0], 0x2c);
        assert_eq!(digest.bytes[1], 0xf2);
    }

    #[test]
    fn test_layer_digest_hex() {
        let digest = LayerDigest::compute(b"");
        let hex = digest.to_hex();
        assert_eq!(hex.len(), 64);
        // SHA-256("") =
        // e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert!(hex.starts_with("e3b0c442"));
    }

    #[test]
    fn test_is_gzip() {
        assert!(is_gzip(&[0x1f, 0x8b, 0x08]));
        assert!(!is_gzip(&[0x00, 0x00]));
        assert!(!is_gzip(&[0x1f]));
    }

    #[test]
    fn test_tar_size_parse() {
        let mut header = [0u8; 512];
        // Octal "0000644" at offset 124
        header[124] = b'0';
        header[125] = b'0';
        header[126] = b'0';
        header[127] = b'0';
        header[128] = b'6';
        header[129] = b'4';
        header[130] = b'4';
        assert_eq!(parse_tar_size(&header), 0o644);
    }

    #[test]
    fn test_container_image_compose() {
        let config = b"config data";
        let layer1 = b"layer 1 data";
        let layer2 = b"layer 2 data";
        let image = ContainerImage::compose("test:latest", config, &[layer1, layer2]);
        assert_eq!(image.name, "test:latest");
        assert_eq!(image.layers.len(), 2);
        assert_eq!(image.manifest.layer_digests.len(), 2);
        assert_eq!(image.manifest.schema_version, 2);
        assert_eq!(image.image_id, image.manifest.config_digest);
    }

    #[test]
    fn test_layer_cache_operations() {
        let mut cache = LayerCache::new(2);
        let digest = LayerDigest::compute(b"test");
        let hex = digest.to_hex();
        let layer = CachedLayer {
            digest: digest.clone(),
            extracted_path: String::from("/layers/test"),
            size_bytes: 1024,
            reference_count: 1,
        };
        assert!(cache.insert(layer));
        assert_eq!(cache.entry_count(), 1);
        assert!(cache.get(&hex).is_some());
        assert!(cache.add_ref(&hex));
        assert_eq!(cache.get(&hex).unwrap().reference_count, 2);
        assert!(cache.release(&hex));
        assert_eq!(cache.get(&hex).unwrap().reference_count, 1);
        assert!(cache.release(&hex)); // drops to 0, removed
        assert_eq!(cache.entry_count(), 0);
    }

    #[test]
    fn test_layer_cache_full() {
        let mut cache = LayerCache::new(1);
        let l1 = CachedLayer {
            digest: LayerDigest::compute(b"a"),
            extracted_path: String::from("/a"),
            size_bytes: 100,
            reference_count: 1,
        };
        let l2 = CachedLayer {
            digest: LayerDigest::compute(b"b"),
            extracted_path: String::from("/b"),
            size_bytes: 200,
            reference_count: 1,
        };
        assert!(cache.insert(l1));
        assert!(cache.is_full());
        assert!(!cache.insert(l2));
    }

    // --- Cgroup Memory Controller tests ---

    #[test]
    fn test_cgroup_memory_basic() {
        let mut mem = CgroupMemoryController::new(1);
        mem.set_hard_limit(4096).unwrap();
        assert!(mem.charge(2048).is_ok());
        assert_eq!(mem.usage_current, 2048);
        assert_eq!(mem.usage_peak, 2048);
        mem.uncharge(1024);
        assert_eq!(mem.usage_current, 1024);
        assert_eq!(mem.usage_peak, 2048); // peak unchanged
    }

    #[test]
    fn test_cgroup_memory_oom() {
        let mut mem = CgroupMemoryController::new(1);
        mem.set_hard_limit(1024).unwrap();
        mem.charge(512).unwrap();
        let result = mem.charge(1024);
        assert!(result.is_err());
        assert_eq!(mem.oom.oom_kill_count, 1);
        assert!(mem.oom.under_oom);
    }

    #[test]
    fn test_cgroup_memory_soft_limit() {
        let mut mem = CgroupMemoryController::new(1);
        mem.set_soft_limit(512);
        mem.charge(256).unwrap();
        assert!(!mem.soft_limit_exceeded());
        // Set hard limit high enough to not OOM
        mem.set_hard_limit(4096).unwrap();
        mem.charge(512).unwrap();
        assert!(mem.soft_limit_exceeded());
    }

    #[test]
    fn test_cgroup_memory_reclaim_from_cache() {
        let mut mem = CgroupMemoryController::new(1);
        mem.set_hard_limit(2048).unwrap();
        mem.charge(1024).unwrap();
        mem.add_cache(512);
        // Now usage_current = 1024 + 512 = 1536, cache = 512
        // Charge 1024 more would exceed 2048, but cache can be reclaimed
        assert!(mem.charge(1024).is_ok());
    }

    #[test]
    fn test_memory_stat_total() {
        let stat = MemoryStat {
            rss: 1000,
            cache: 500,
            mapped_file: 200,
            anon: 300,
            swap: 0,
        };
        assert_eq!(stat.total(), 1500);
    }

    // --- Cgroup CPU Controller tests ---

    #[test]
    fn test_cgroup_cpu_shares() {
        let mut cpu = CgroupCpuController::new(1);
        assert_eq!(cpu.shares, 1024);
        cpu.set_shares(2048).unwrap();
        assert_eq!(cpu.shares, 2048);
        assert!(cpu.set_shares(1).is_err()); // below minimum
        assert!(cpu.set_shares(300000).is_err()); // above maximum
    }

    #[test]
    fn test_cgroup_cpu_bandwidth() {
        let mut cpu = CgroupCpuController::new(1);
        cpu.set_bandwidth(50000, 100000).unwrap();
        assert_eq!(cpu.quota_us, 50000);
        assert_eq!(cpu.period_us, 100000);
        // 50% CPU
        assert_eq!(cpu.effective_cpu_percent_x100(), 5000);
    }

    #[test]
    fn test_cgroup_cpu_throttle() {
        let mut cpu = CgroupCpuController::new(1);
        cpu.set_bandwidth(10000, 100000).unwrap(); // 10% CPU
                                                   // 10000us = 10_000_000ns quota
        assert!(!cpu.consume_runtime(5_000_000)); // 5ms, not throttled
        assert!(cpu.consume_runtime(6_000_000)); // 6ms more, total 11ms > 10ms quota
        assert!(cpu.throttled);
        assert_eq!(cpu.stats.nr_throttled, 1);
    }

    #[test]
    fn test_cgroup_cpu_period_reset() {
        let mut cpu = CgroupCpuController::new(1);
        cpu.set_bandwidth(10000, 100000).unwrap();
        cpu.consume_runtime(10_000_000);
        cpu.new_period();
        assert!(!cpu.throttled);
        assert_eq!(cpu.stats.nr_periods, 1);
    }

    #[test]
    fn test_cgroup_cpu_burst() {
        let mut cpu = CgroupCpuController::new(1);
        cpu.set_bandwidth(10000, 100000).unwrap(); // 10ms quota
        cpu.set_burst(5000); // 5ms burst allowed
                             // Use only 5ms of 10ms quota -> 5ms unused
        cpu.consume_runtime(5_000_000);
        cpu.new_period(); // 5ms saved as burst budget
        assert!(cpu.burst_budget_ns > 0);
        // Now can use up to 15ms (10ms quota + 5ms burst)
        assert!(!cpu.consume_runtime(14_000_000));
    }

    #[test]
    fn test_cgroup_cpu_proportional() {
        let cpu = CgroupCpuController::new(1);
        // Default shares=1024, total=2048 -> 50% of period
        let ns = cpu.proportional_runtime_ns(2048);
        // period=100000us=100_000_000ns, 1024/2048 = 50% = 50_000_000ns
        assert_eq!(ns, 50_000_000);
    }

    // --- Overlay Filesystem tests ---

    #[test]
    fn test_overlay_basic_lookup() {
        let mut lower = OverlayLayer::new(true);
        // Add entry directly (bypass readonly check for setup)
        lower.entries.insert(
            String::from("etc/passwd"),
            OverlayEntry {
                path: String::from("etc/passwd"),
                kind: OverlayEntryKind::File,
                content: b"root:x:0:0".to_vec(),
                mode: 0o644,
            },
        );

        let mut fs = OverlayFs::new("/tmp/work");
        fs.add_lower_layer(lower);
        let entry = fs.lookup("etc/passwd");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().content, b"root:x:0:0");
    }

    #[test]
    fn test_overlay_upper_takes_precedence() {
        let mut lower = OverlayLayer::new(true);
        lower.entries.insert(
            String::from("etc/hostname"),
            OverlayEntry {
                path: String::from("etc/hostname"),
                kind: OverlayEntryKind::File,
                content: b"oldhost".to_vec(),
                mode: 0o644,
            },
        );
        let mut fs = OverlayFs::new("/tmp/work");
        fs.add_lower_layer(lower);
        fs.write_file("etc/hostname", b"newhost".to_vec(), 0o644)
            .unwrap();
        let entry = fs.lookup("etc/hostname").unwrap();
        assert_eq!(entry.content, b"newhost");
    }

    #[test]
    fn test_overlay_whiteout() {
        let mut lower = OverlayLayer::new(true);
        lower.entries.insert(
            String::from("etc/shadow"),
            OverlayEntry {
                path: String::from("etc/shadow"),
                kind: OverlayEntryKind::File,
                content: b"secret".to_vec(),
                mode: 0o600,
            },
        );
        let mut fs = OverlayFs::new("/tmp/work");
        fs.add_lower_layer(lower);
        assert!(fs.lookup("etc/shadow").is_some());
        fs.delete_file("etc/shadow").unwrap();
        assert!(fs.lookup("etc/shadow").is_none());
    }

    #[test]
    fn test_overlay_opaque_dir() {
        let mut lower = OverlayLayer::new(true);
        lower.entries.insert(
            String::from("etc/conf.d/old.conf"),
            OverlayEntry {
                path: String::from("etc/conf.d/old.conf"),
                kind: OverlayEntryKind::File,
                content: b"old".to_vec(),
                mode: 0o644,
            },
        );
        let mut fs = OverlayFs::new("/tmp/work");
        fs.add_lower_layer(lower);
        fs.make_opaque_dir("etc/conf.d").unwrap();
        // The lower layer file should not be visible via listing
        let listing = fs.list_dir("etc/conf.d");
        assert!(listing.is_empty());
    }

    #[test]
    fn test_overlay_readonly_layer() {
        let mut lower = OverlayLayer::new(true);
        let result = lower.add_entry(OverlayEntry {
            path: String::from("test"),
            kind: OverlayEntryKind::File,
            content: Vec::new(),
            mode: 0o644,
        });
        assert!(result.is_err());
    }

    // --- Veth Networking tests ---

    #[test]
    fn test_veth_pair_creation() {
        let pair = create_veth_pair("veth0", "eth0", 42);
        assert_eq!(pair.host.name, "veth0");
        assert_eq!(pair.container.name, "eth0");
        assert_eq!(pair.host.peer_name, "eth0");
        assert_eq!(pair.container.peer_name, "veth0");
        assert_eq!(pair.host.namespace_id, 0);
        assert_eq!(pair.container.namespace_id, 42);
        assert_eq!(pair.host.mtu, 1500);
        // MACs differ
        assert_ne!(pair.host.mac, pair.container.mac);
    }

    #[test]
    fn test_veth_mac_generation() {
        let mac1 = generate_veth_mac(1);
        let mac2 = generate_veth_mac(2);
        assert_eq!(mac1[0], 0x02); // locally administered
        assert_ne!(mac1, mac2);
    }

    #[test]
    fn test_nat_table() {
        let mut nat = NatTable::new(0xC0A80001); // 192.168.0.1
        nat.enable_masquerade();
        assert!(nat.masquerade_enabled);

        let mapping = NatPortMapping {
            external_port: 8080,
            internal_port: 80,
            protocol: 6,              // TCP
            container_ip: 0x0A000002, // 10.0.0.2
        };
        nat.add_port_mapping(mapping).unwrap();
        assert_eq!(nat.port_mappings.len(), 1);

        // Duplicate should fail
        let dup = NatPortMapping {
            external_port: 8080,
            internal_port: 8080,
            protocol: 6,
            container_ip: 0x0A000003,
        };
        assert!(nat.add_port_mapping(dup).is_err());

        // Lookup
        let found = nat.lookup_inbound(8080, 6).unwrap();
        assert_eq!(found.internal_port, 80);
        assert_eq!(found.container_ip, 0x0A000002);

        // SNAT rewrite
        let rewritten = nat.snat_rewrite(0x0A000002);
        assert_eq!(rewritten, Some(0xC0A80001));

        // Remove
        assert!(nat.remove_port_mapping(8080, 6));
        assert!(nat.lookup_inbound(8080, 6).is_none());
    }

    #[test]
    fn test_veth_bridge() {
        let mut bridge = VethBridge::new("br0", 0x0A000001, 0xFFFFFF00);
        bridge.attach("veth0");
        bridge.attach("veth1");
        assert_eq!(bridge.attached_count(), 2);
        assert!(bridge.in_subnet(0x0A0000FE)); // 10.0.0.254
        assert!(!bridge.in_subnet(0x0B000001)); // 11.0.0.1

        bridge.add_arp_proxy(ArpProxyEntry {
            ip: 0x0A000002,
            mac: [0x02, 0x42, 0x00, 0x00, 0x00, 0x01],
        });
        assert!(bridge.arp_lookup(0x0A000002).is_some());
        assert!(bridge.arp_lookup(0x0A000003).is_none());

        bridge.detach("veth0");
        assert_eq!(bridge.attached_count(), 1);
    }

    // --- Seccomp BPF tests ---

    #[test]
    fn test_seccomp_data_as_bytes() {
        let data = SeccompData::new(1, audit_arch::X86_64, [0; 6]);
        let bytes = data.as_bytes();
        assert_eq!(bytes.len(), 64);
        // nr=1 at offset 0
        assert_eq!(
            u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            1
        );
        // arch at offset 4
        assert_eq!(
            u32::from_ne_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            audit_arch::X86_64
        );
    }

    #[test]
    fn test_seccomp_filter_validate() {
        let mut filter = SeccompFilter::new();
        assert!(filter.validate().is_err()); // empty

        filter.push(BpfInstruction::ret(SeccompAction::Allow as u32));
        assert!(filter.validate().is_ok());
    }

    #[test]
    fn test_seccomp_filter_deny_syscalls() {
        let filter = SeccompFilter::deny_syscalls(
            audit_arch::X86_64,
            &[59], // deny execve
            1,     // EPERM
        );
        assert!(filter.validate().is_ok());

        // Test: execve should be denied
        let data_execve = SeccompData::new(59, audit_arch::X86_64, [0; 6]);
        let action = filter.evaluate(&data_execve);
        assert_eq!(action, SeccompAction::errno(1));

        // Test: read should be allowed
        let data_read = SeccompData::new(0, audit_arch::X86_64, [0; 6]);
        let action = filter.evaluate(&data_read);
        assert_eq!(action, SeccompAction::Allow as u32);
    }

    #[test]
    fn test_seccomp_filter_allow_syscalls() {
        let filter = SeccompFilter::allow_syscalls(
            audit_arch::X86_64,
            &[0, 1, 60], // read, write, exit
        );
        assert!(filter.validate().is_ok());

        let data_read = SeccompData::new(0, audit_arch::X86_64, [0; 6]);
        assert_eq!(filter.evaluate(&data_read), SeccompAction::Allow as u32);

        let data_execve = SeccompData::new(59, audit_arch::X86_64, [0; 6]);
        assert_eq!(
            filter.evaluate(&data_execve),
            SeccompAction::KillProcess as u32
        );
    }

    #[test]
    fn test_seccomp_wrong_arch_killed() {
        let filter = SeccompFilter::deny_syscalls(audit_arch::X86_64, &[59], 1);
        let data = SeccompData::new(59, audit_arch::AARCH64, [0; 6]);
        assert_eq!(filter.evaluate(&data), SeccompAction::KillProcess as u32);
    }

    #[test]
    fn test_seccomp_state_disabled() {
        let state = SeccompState::new();
        let data = SeccompData::new(59, audit_arch::X86_64, [0; 6]);
        assert_eq!(state.evaluate(&data), SeccompAction::Allow as u32);
    }

    #[test]
    fn test_seccomp_state_strict() {
        let mut state = SeccompState::new();
        state.mode = SeccompMode::Strict;
        // read(0) allowed
        let data = SeccompData::new(0, audit_arch::X86_64, [0; 6]);
        assert_eq!(state.evaluate(&data), SeccompAction::Allow as u32);
        // execve(59) killed
        let data2 = SeccompData::new(59, audit_arch::X86_64, [0; 6]);
        assert_eq!(state.evaluate(&data2), SeccompAction::KillThread as u32);
    }

    #[test]
    fn test_seccomp_state_filter_install() {
        let mut state = SeccompState::new();
        let filter = SeccompFilter::deny_syscalls(audit_arch::X86_64, &[59], 1);
        state.install_filter(filter).unwrap();
        assert_eq!(state.mode, SeccompMode::Filter);
        assert_eq!(state.filter_count(), 1);
    }

    #[test]
    fn test_seccomp_fork_inherit() {
        let mut state = SeccompState::new();
        let mut f1 = SeccompFilter::deny_syscalls(audit_arch::X86_64, &[59], 1);
        f1.inherit_on_fork = true;
        let mut f2 = SeccompFilter::deny_syscalls(audit_arch::X86_64, &[60], 1);
        f2.inherit_on_fork = false;
        state.install_filter(f1).unwrap();
        state.install_filter(f2).unwrap();
        let child = state.fork_inherit();
        assert_eq!(child.filter_count(), 1); // only inherited one
    }

    #[test]
    fn test_bpf_instruction_constructors() {
        let load = BpfInstruction::load_word(4);
        assert_eq!(load.code, BpfOpcode::LdAbsW as u16);
        assert_eq!(load.k, 4);

        let jeq = BpfInstruction::jump_eq(42, 1, 0);
        assert_eq!(jeq.code, BpfOpcode::JmpJeqK as u16);
        assert_eq!(jeq.k, 42);
        assert_eq!(jeq.jt, 1);
        assert_eq!(jeq.jf, 0);

        let ret = BpfInstruction::ret(SeccompAction::Allow as u32);
        assert_eq!(ret.code, BpfOpcode::Ret as u16);
    }

    #[test]
    fn test_seccomp_errno_action() {
        let action = SeccompAction::errno(13); // EACCES
        assert_eq!(action, 0x0005_000D);
    }

    // --- Helper tests ---

    #[test]
    fn test_parse_u32() {
        assert_eq!(parse_u32("12345"), Some(12345));
        assert_eq!(parse_u32("0"), Some(0));
        assert_eq!(parse_u32("abc"), None);
        assert_eq!(parse_u32(""), Some(0));
    }

    #[test]
    fn test_parse_u64() {
        assert_eq!(parse_u64("123456789"), Some(123456789));
        assert_eq!(parse_u64("0"), Some(0));
    }

    #[test]
    fn test_sha256_empty() {
        let hash = simple_sha256(b"");
        // SHA-256("") =
        // e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert_eq!(hash[0], 0xe3);
        assert_eq!(hash[1], 0xb0);
        assert_eq!(hash[2], 0xc4);
        assert_eq!(hash[3], 0x42);
    }

    #[test]
    fn test_sha256_hello() {
        let hash = simple_sha256(b"hello");
        // SHA-256("hello") =
        // 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        assert_eq!(hash[0], 0x2c);
        assert_eq!(hash[1], 0xf2);
        assert_eq!(hash[2], 0x4d);
        assert_eq!(hash[3], 0xba);
    }
}
