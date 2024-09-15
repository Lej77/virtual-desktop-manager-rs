fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_none() {
        panic!("This program can only be compiled for Windows");
    }

    let icon_path = "src/icons/edges - transparent with white.ico";
    let manifest_path = "virtual-desktop-manager.exe.manifest";

    println!("cargo::rerun-if-changed=\"Cargo.toml\"");
    println!("cargo::rerun-if-changed=\"{icon_path}\"");
    println!("cargo::rerun-if-changed=\"{manifest_path}\"");

    // This sets the icon for the executable and also embeds a manifest so that
    // we can link to the GUI libraries, see
    // https://github.com/gabdube/native-windows-gui/issues/241
    //
    // Without the manifest the program fails to start with exit code:
    // 0xc0000139, STATUS_ENTRYPOINT_NOT_FOUND
    winres::WindowsResource::new()
        .set_icon(icon_path)
        .set_manifest_file(manifest_path)
        .compile()
        .unwrap();
}
