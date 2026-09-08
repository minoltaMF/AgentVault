fn main() {
    println!("cargo:rerun-if-env-changed=AGENTVAULT_RELEASE_REPOSITORY");

    #[cfg(feature = "desktop")]
    {
        println!("cargo:rerun-if-changed=tauri.conf.json");
        println!("cargo:rerun-if-changed=icons/32x32.png");
        println!("cargo:rerun-if-changed=icons/128x128.png");
        println!("cargo:rerun-if-changed=icons/128x128@2x.png");
        println!("cargo:rerun-if-changed=icons/icon.icns");
        println!("cargo:rerun-if-changed=icons/icon.ico");
        tauri_build::build()
    }
}
