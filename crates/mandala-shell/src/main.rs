use mandala_core::paths::HostPaths;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let paths = HostPaths::from_env();
    // Headless check for CI and for verifying a newly signed module set.
    if std::env::args().skip(1).any(|a| a == "--selftest") {
        return mandala_shell::selftest(paths);
    }
    mandala_shell::run(paths)
}
