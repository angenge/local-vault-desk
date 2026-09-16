use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("缺少 CARGO_MANIFEST_DIR"));
    
    // 静态链接 librclone.a
    let lib_dir = manifest_dir.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=rclone");

    // Go 运行时底层依赖库
    println!("cargo:rustc-link-lib=dylib=ws2_32");
    println!("cargo:rustc-link-lib=dylib=userenv");
    println!("cargo:rustc-link-lib=dylib=iphlpapi");
    println!("cargo:rustc-link-lib=dylib=winmm");
    println!("cargo:rustc-link-lib=dylib=bcrypt");
    println!("cargo:rustc-link-lib=dylib=ntdll");
    println!("cargo:rustc-link-lib=dylib=advapi32");
    println!("cargo:rustc-link-lib=dylib=shell32");
    println!("cargo:rustc-link-lib=dylib=ole32");

    // MSVC 链接器下补充 legacy stdio 符号（解决 Go cgo 生成的代码引用的 fprintf 等历史符号）
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("msvc") {
        println!("cargo:rustc-link-lib=dylib=legacy_stdio_definitions");
    }

    println!("cargo:rerun-if-changed=lib/librclone.a");
    println!("cargo:rerun-if-changed=../dist");
    println!("cargo:rerun-if-changed=tauri.conf.json");

    tauri_build::build();
}
