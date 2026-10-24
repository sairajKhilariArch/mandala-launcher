//! Minimal process introspection for the performance budget in the roadmap
//! (cold boot < 20 ms, idle RSS < 5 MB). Numbers are reported, never assumed.

/// Resident set size of the current process in KiB, when the OS exposes it.
///
/// Linux: `/proc/self/status` `VmRSS`. Other platforms return `None`; the
/// shell then simply omits the figure.
pub fn process_rss_kb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        parse_vm_rss(&status)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Extract `VmRSS: <n> kB` from the text of `/proc/self/status`.
pub fn parse_vm_rss(status: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let rest = line.strip_prefix("VmRSS:")?;
        rest.split_whitespace().next()?.parse().ok()
    })
}

/// Peak RSS (`VmHWM`) in KiB, where available.
pub fn process_peak_rss_kb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        status.lines().find_map(|line| {
            let rest = line.strip_prefix("VmHWM:")?;
            rest.split_whitespace().next()?.parse().ok()
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vm_rss_lines() {
        let sample = "Name:\tmandala-shell\nVmPeak:\t  123456 kB\nVmRSS:\t   4321 kB\nVmHWM:\t   5000 kB\n";
        assert_eq!(parse_vm_rss(sample), Some(4321));
        assert_eq!(parse_vm_rss("nothing here"), None);
        assert_eq!(parse_vm_rss("VmRSS: not-a-number kB"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_process_reports_rss() {
        let kb = process_rss_kb().expect("linux exposes VmRSS");
        assert!(kb > 0);
        assert!(process_peak_rss_kb().unwrap() >= kb);
    }
}
