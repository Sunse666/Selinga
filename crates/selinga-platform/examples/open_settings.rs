fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        let base = std::env::var("APPDATA").unwrap_or_default();
        format!("{base}\\selinga\\settings.toml")
    });
    let path = std::path::Path::new(&path);
    println!("path: {}", path.display());
    println!("exists: {}", path.exists());
    match selinga_platform::open_in_editor(path) {
        Ok(()) => println!("open_in_editor: OK"),
        Err(err) => println!("open_in_editor: FAILED: {err}"),
    }
}
