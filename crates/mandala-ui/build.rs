fn main() {
    println!("cargo:rerun-if-changed=ui");
    slint_build::compile("ui/main_window.slint").expect("failed to compile Slint UI");
}
