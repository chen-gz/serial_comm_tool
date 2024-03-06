fn main() {
    slint_build::compile("ui/appwindow.slint").unwrap();
    // let config =
    // slint_build::CompilerConfiguration::new()
    // .with_style("material".into());
    // slint_build::compile_with_config("ui/appwindow.slint", config).unwrap();
}
