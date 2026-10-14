fn main() {
    println!("cargo::rerun-if-changed=../../assets/wyrmyon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../../assets/wyrmyon.ico");
        resource
            .compile()
            .expect("the Windows icon compiles into the binaries");
    }
}
