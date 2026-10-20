fn main() {
    // Só o .exe do Windows leva o ícone embutido; no Linux ele vem do .desktop.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=pacote/featherlauncher.rc");
        println!("cargo:rerun-if-changed=pacote/icones/featherlauncher.ico");
        embed_resource::compile("pacote/featherlauncher.rc", embed_resource::NONE).manifest_optional().expect("ícone do .exe");
    }
}
