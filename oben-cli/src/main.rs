fn main() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(oben_cli::dispatch::run_cli())?;
    Ok(())
}
