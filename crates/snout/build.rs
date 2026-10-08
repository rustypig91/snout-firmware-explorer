fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/snout.rc");
    println!("cargo:rerun-if-changed=packaging/icons/snout.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("packaging/windows/snout.rc", embed_resource::NONE)
            .manifest_required()
            .expect("compile the Snout Windows icon resource");
    }
}
