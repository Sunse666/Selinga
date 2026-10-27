fn main() {
    println!("cargo:rerun-if-changed=assets/selinga.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/selinga.ico");
    if let Err(err) = resource.compile() {
        println!("cargo:warning=icon resource not embedded: {err}");
    }
}
