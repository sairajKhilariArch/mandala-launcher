use mandala_core::paths::HostPaths;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    mandala_shell::run(HostPaths::from_env())
}
