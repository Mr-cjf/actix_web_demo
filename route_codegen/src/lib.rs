extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use std::fs;
use std::path::{Path, PathBuf};
use syn::ext::IdentExt;
use syn::LitStr;
use syn::{parse_file, ItemFn};
// 新增: 为Ident类型启用.unraw()方法

#[proc_macro]
pub fn generate_configure(_input: TokenStream) -> TokenStream {
    let functions = scan_crate_for_route_functions();

    println!("🔍 Found {} route functions", functions.len());
    for func in &functions {
        println!(" - {} [{} {}]", func.name, func.method, func.route_path);
    }

    let services = functions.iter().map(|f| {
        let ident = syn::Ident::new(&f.name, proc_macro2::Span::call_site());

        quote! {
            cfg.service(crate::handler::nation::#ident);
        }
    });

    let expanded = quote! {
        pub fn configure(cfg: &mut actix_web::web::ServiceConfig) {
            #(#services)*
        }
    };

    // 打印最终生成的代码字符串（用于调试）
    #[cfg(debug_assertions)]
    {
        let generated_code = expanded.to_string();
        println!("🧾 Generated code:\n{}", generated_code);
    }

    TokenStream::from(expanded)
}

fn scan_crate_for_route_functions() -> Vec<RouteFunction> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR environment variable not found");
    let src_path = PathBuf::from(manifest_dir).join("src");
    let mut result = Vec::new();

    if let Some(main_path) = find_main_or_lib(&src_path) {
        process_file(&main_path, &mut result);
    }

    scan_directory(&src_path, &["main.rs", "lib.rs", "mod.rs"], &mut result);

    let handler_path = src_path.join("handler");
    if handler_path.exists() {
        scan_directory(&handler_path, &["mod.rs"], &mut result);
    }

    result
}

fn scan_directory<P: AsRef<Path>>(
    path: P,
    exclude_files: &[&str],
    result: &mut Vec<RouteFunction>,
) {
    let path = path.as_ref();
    println!("📁 Scanning directory: {:?}", path);
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            let file_name = entry_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");

            if entry_path.is_file() {
                let ext = entry_path.extension().and_then(|s| s.to_str());
                if ext == Some("rs") && !exclude_files.contains(&file_name) {
                    process_file(&entry_path, result);
                }
            } else if entry_path.is_dir() {
                scan_directory(&entry_path, exclude_files, result);
            }
        }
    } else {
        eprintln!("❌ Failed to read directory: {:?}", path);
    }
}

fn process_file(path: &Path, result: &mut Vec<RouteFunction>) {
    if let Ok(content) = fs::read_to_string(path) {
        #[cfg(debug_assertions)]
        {
            let first_100 = content.chars().take(100).collect::<String>();
            println!("📄 File content (first 100 chars): {:?}", first_100);
        }

        scan_file(&content, result);
    } else {
        eprintln!("❌ Failed to read file: {}", path.display());
    }
}

fn scan_file(content: &str, result: &mut Vec<RouteFunction>) {
    match parse_file(content) {
        Ok(file) => {
            for item in file.items {
                if let syn::Item::Fn(fn_item) = item {
                    if let Some(route_fn) = extract_route_info(&fn_item) {
                        println!("✅ Found route function: {}", route_fn.name);
                        result.push(route_fn);
                    }
                }
            }
        }
        Err(e) => {
            let msg = e.to_string();
            panic!("❌ Failed to parse file: {}", msg);
        }
    }
}

struct RouteFunction {
    name: String,
    method: String,
    route_path: String,
}

fn extract_route_info(fn_item: &ItemFn) -> Option<RouteFunction> {
    use once_cell::sync::Lazy;
    use std::collections::HashMap;

    static METHOD_MAP: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
        [
            ("get", "get"),
            ("post", "post"),
            ("put", "put"),
            ("delete", "delete"),
            ("head", "head"),
            ("connect", "connect"),
            ("options", "options"),
            ("trace", "trace"),
            ("patch", "patch"),
        ]
            .iter()
            .cloned()
            .collect()
    });

    let mut method = None;
    let mut path = None;

    for attr in &fn_item.attrs {
        let ident = attr.path().get_ident()?;
        if let Some(m) = METHOD_MAP.get(&ident.unraw().to_string()) {
            if let Ok(lit_str) = attr.parse_args::<LitStr>() {
                method = Some(m.to_string());
                path = Some(lit_str.value());
            }
        }
    }

    if method.is_none() || path.is_none() {
        return None;
    }

    let name = fn_item.sig.ident.to_string();

    Some(RouteFunction {
        name,
        method: method?,
        route_path: path?,
    })
}

fn find_main_or_lib(src_path: &Path) -> Option<PathBuf> {
    let main_rs = src_path.join("main.rs");
    let lib_rs = src_path.join("lib.rs");

    if main_rs.exists() {
        Some(main_rs)
    } else if lib_rs.exists() {
        Some(lib_rs)
    } else {
        None
    }
}
