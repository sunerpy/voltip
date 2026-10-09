fn main() {
    // The page calls the back plugin's commands itself (src/back.rs): an app's own plugin gets
    // permissions only for the commands named here, and capabilities/default.json allows them.
    let back = tauri_build::InlinedPlugin::new().commands(&["register_listener", "remove_listener", "release"]);
    if let Err(error) = tauri_build::try_build(tauri_build::Attributes::new().plugin("voltip-back", back)) {
        println!("{error:#}");
        std::process::exit(1);
    }
}
