fn main() {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("wallpaper-host runtime initialization failed: {error}");
            std::process::exit(1);
        }
    };

    if let Err(error) = runtime.block_on(wallpaper_ipc::server::run()) {
        eprintln!("wallpaper-host IPC server failed: {error}");
        std::process::exit(1);
    }
}
