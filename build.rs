fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(feature = "app")]
    embed_windows_resources();
}

#[cfg(feature = "app")]
fn embed_windows_resources() {
    use std::env::var;
    println!("cargo:rerun-if-changed=assets/chronos.rc");
    println!("cargo:rerun-if-changed=assets/chronos.ico");
    let macros = [
        format!("VERSION_MAJOR={}", var("CARGO_PKG_VERSION_MAJOR").unwrap()),
        format!("VERSION_MINOR={}", var("CARGO_PKG_VERSION_MINOR").unwrap()),
        format!("VERSION_PATCH={}", var("CARGO_PKG_VERSION_PATCH").unwrap()),
        format!("VERSION_TEXT=\"{}\"", var("CARGO_PKG_VERSION").unwrap()),
    ];
    embed_resource::compile_for(
        "assets/chronos.rc",
        ["chronos"],
        embed_resource::ParamsMacrosAndIncludeDirs(macros, ["assets"]),
    )
    .manifest_required()
    .unwrap();
}
