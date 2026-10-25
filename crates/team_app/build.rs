//! With the `tauri` feature: the Tauri build step, reading the packaged
//! config in `app/tauri.conf.json`. Without it: nothing.

fn main() {
    println!("cargo:rerun-if-changed=../../app/tauri.conf.json");
    #[cfg(feature = "tauri")]
    tauri_build::try_build(tauri_build::Attributes::new().config_path("../../app/tauri.conf.json"))
        .expect("tauri build script");
}
