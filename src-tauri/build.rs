fn main() {
    if matches!(
        std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("windows" | "macos")
    ) {
        tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "start",
                "stop",
                "status",
                "account",
                "set_key",
                "forget_key",
            ]),
        ))
        .expect("generate app resources and permissions");
    }
}
