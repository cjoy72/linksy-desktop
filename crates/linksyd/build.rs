//! Windows: Linksy's icon and name in the program itself, for File Explorer, Task Manager and
//! the sign-in list.

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/windows/linksy.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/windows/linksy.ico").set("FileDescription", "Linksy").set("ProductName", "Linksy");
        if let Err(e) = res.compile() {
            println!("cargo:warning=no icon in linksyd.exe: {e}");
        }
    }
}
