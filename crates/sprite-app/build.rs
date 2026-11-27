fn main() {
    #[cfg(target_os = "windows")]
    windows_build();
}

#[cfg(target_os = "windows")]
fn windows_build() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=../../packaging/windows/sprite.rc");
    println!("cargo:rerun-if-changed=../../packaging/windows/sprite.ico");
    if let Err(error) = embed_resource::compile_for(
        "../../packaging/windows/sprite.rc",
        ["sprite"],
        embed_resource::NONE,
    )
    .manifest_required()
    {
        println!("cargo::error={error}");
        std::process::exit(1);
    }
}
