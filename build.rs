fn main() {
    println!("cargo:rerun-if-changed=assets/aire.ico");
    let mut recursos = winresource::WindowsResource::new();
    recursos
        .set_icon("assets/aire.ico")
        .set("ProductName", "Aire Control")
        .set("FileDescription", "Control del aire del cuarto");
    recursos.compile().expect("No se pudieron agregar el ícono y los datos del programa");
}
