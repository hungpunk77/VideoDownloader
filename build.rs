fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icons/icon.ico");
    println!("cargo:rerun-if-changed=assets/videodownloader.manifest");

    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icons/icon.ico");
        res.set_manifest_file("assets/videodownloader.manifest");

        // Keep PE VersionInfo strings strictly 7-bit ASCII so llvm-rc works on Linux cross-builds.
        res.set("ProductName", "VideoDownloader");
        res.set(
            "FileDescription",
            "VideoDownloader - Desktop Video and Music Downloader",
        );
        res.set("CompanyName", "VideoDownloader");
        res.set("LegalCopyright", "VideoDownloader Contributors");
        res.set("OriginalFilename", "videodownloader.exe");
        res.set("InternalName", "videodownloader");

        if let Err(err) = res.compile() {
            eprintln!("Windows resource compilation failed: {err}");
            eprintln!("Ensure llvm-rc (from the 'llvm' package) is installed and in PATH when cross-compiling.");
            std::process::exit(1);
        }
    }
}
