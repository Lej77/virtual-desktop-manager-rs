fn main() {
    // This sets the icon for the executable and also embeds a manifest so that
    // we can link to the GUI libraries, see
    // https://github.com/gabdube/native-windows-gui/issues/241
    //
    // Without the manifest the program fails to start with exit code:
    // 0xc0000139, STATUS_ENTRYPOINT_NOT_FOUND
    embed_resource::compile("virtual-desktop-manager-manifest.rc", embed_resource::NONE);
}
