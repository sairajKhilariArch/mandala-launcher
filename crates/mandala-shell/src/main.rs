use mandala_core::paths::HostPaths;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let paths = HostPaths::from_env();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Headless check for CI and for verifying a newly signed module set.
    if args.iter().any(|a| a == "--selftest") {
        return mandala_shell::selftest(paths);
    }
    // Real window, but exit after the first interactive frame: measures the
    // full boot path (window + renderer + first paint) without needing input.
    if args.iter().any(|a| a == "--boot-report") {
        return mandala_shell::boot_report(paths);
    }
    mandala_shell::run(paths)
}
