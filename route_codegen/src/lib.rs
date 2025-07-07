extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
// 用于解析 Rust 源码为 AST
use rayon::prelude::*;
// 用于生成 Rust 代码的宏
use std::collections::HashMap;
// 解析 Cargo.toml 使用
use std::fs;
// 文件系统操作
use std::io::Read;
// 文件读取
use std::path::{Path, PathBuf};
// 路径处理
use syn::LitStr;
// 用于解析属性中的字符串字面量
use syn::{parse_file, ItemFn};
// 并行迭代支持

/// generate_configure 是一个过程宏，它会扫描整个项目和 workspace 成员中的路由函数，
/// 然后自动生成 configure 函数来注册这些路由。
///
/// 它是通过 #[proc_macro] 注册的过程宏，供其他模块使用：
///
/// ```rust
/*use crate::generate_configure;


generate_configure!();

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    unsafe {
        env::set_var("RUST_LOG", "actix_web=info");
    }
    env_logger::init();

    println!("Starting HTTP server at http://127.0.0.1:8080");

    HttpServer::new(|| App::new().configure(configure))
        .bind("127.0.0.1:8080")?
        .run()
        .await
}*/

/// ```
#[proc_macro]
pub fn generate_configure(_input: TokenStream) -> TokenStream {
    let functions = scan_crate_for_route_functions();

    println!("🔍 Found {} route functions", functions.len());
    for func in &functions {
        println!(" - {} [{} {}]", func.name, func.method, func.route_path);
    }

    // 构建服务注册语句
    let services = functions.iter().map(|f| {
        let ident = syn::Ident::new(&f.name, proc_macro2::Span::call_site());
        // 否则只添加标准的 service 注册语句
        quote! {
            cfg.service(crate::handler::nation::#ident);
        }
    });

    // 构建最终的 configure 函数代码
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

/// 扫描当前 crate 中所有的路由函数
fn scan_crate_for_route_functions() -> Vec<RouteFunction> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR environment variable not found");

    let mut result = Vec::new();

    // 先扫描主项目
    scan_project(&manifest_dir, &mut result);

    // 再检查是否为 workspace，并扫描成员项目
    if let Some(workspace_config) = read_workspace_config(&manifest_dir) {
        if let Some(members) = workspace_config.members {
            let workspace_dir = PathBuf::from(&manifest_dir);
            scan_workspace_members(workspace_dir, members, &mut result);
        }
    }

    result
}

/// 遍历 workspace 成员并扫描每个成员项目的源码
fn scan_workspace_members(
    workspace_dir: PathBuf,
    members: Vec<String>,
    result: &mut Vec<RouteFunction>,
) {
    for member in members {
        let member_dir = workspace_dir.join(member);
        if !member_dir.exists() {
            continue;
        }

        let member_manifest_path = member_dir.join("Cargo.toml");
        if !member_manifest_path.exists() {
            continue;
        }

        let member_manifest_dir = member_dir.to_str().unwrap().to_string();
        scan_project(&member_manifest_dir, result);
    }
}

/// 扫描指定项目的 src/ 目录下的所有路由函数
fn scan_project(manifest_dir: &str, result: &mut Vec<RouteFunction>) {
    let src_path = PathBuf::from(manifest_dir).join("src");

    let main_or_lib_path = match find_main_or_lib(&src_path) {
        Some(path) => path,
        None => return,
    };

    // 主文件所在目录
    let root_dir = main_or_lib_path.parent().unwrap_or(&src_path);

    // 排除主文件本身 + mod.rs
    let file_name_to_exclude = main_or_lib_path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| vec![s, "mod.rs"])
        .unwrap_or_else(|| vec!["mod.rs"]);

    scan_directory(root_dir, &file_name_to_exclude[..], result);
}

// 读取 Cargo.toml 中的 workspace 配置
#[derive(Debug)]
struct WorkspaceConfig {
    members: Option<Vec<String>>,
}

/// 读取并解析当前项目的 Cargo.toml，提取其中的 workspace 配置
fn read_workspace_config(manifest_dir: &str) -> Option<WorkspaceConfig> {
    use toml::Value;

    let mut path = PathBuf::from(manifest_dir);
    path.push("Cargo.toml");

    let mut file = fs::File::open(path).ok()?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).ok()?;

    let cargo_toml: HashMap<String, Value> = toml::from_str(&contents).ok()?;
    let workspace_val = cargo_toml.get("workspace")?;
    let members_val = workspace_val.get("members")?;

    if let Some(Value::Array(members)) = Some(members_val) {
        let mut members_vec = Vec::new();
        for member in members {
            if let Some(member_str) = member.as_str() {
                members_vec.push(member_str.to_string());
            }
        }
        return Some(WorkspaceConfig {
            members: if members_vec.is_empty() {
                None
            } else {
                Some(members_vec)
            },
        });
    }

    None
}

/// 查找项目入口文件 main.rs 或 lib.rs
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

/// 递归扫描指定目录中的 .rs 源文件
fn scan_directory<P: AsRef<Path>>(
    path: P,
    exclude_files: &[&str],
    result: &mut Vec<RouteFunction>,
) {
    let path = path.as_ref();
    #[cfg(debug_assertions)]
    println!("📁 Scanning directory: {:?}", path);

    if let Ok(entries) = fs::read_dir(path) {
        let entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();

        let local_results: Vec<_> = entries
            .into_par_iter()
            .filter_map(|entry| {
                let entry_path = entry.path();
                let file_name = entry_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");

                if entry_path.is_file() {
                    let ext = entry_path.extension().and_then(|s| s.to_str());
                    if ext == Some("rs") && !exclude_files.contains(&file_name) {
                        let mut sub_result = Vec::new();
                        process_file(&entry_path, &mut sub_result);
                        return Some(sub_result);
                    }
                } else if entry_path.is_dir() {
                    let mut sub_result = Vec::new();
                    scan_directory(&entry_path, exclude_files, &mut sub_result);
                    return Some(sub_result);
                }

                None
            })
            .flatten()
            .collect();

        result.extend(local_results);
    } else {
        eprintln!("❌ Failed to read directory: {:?}", path);
    }
}

/// 处理单个 .rs 文件，提取其中的路由函数信息
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

/// 将 Rust 源码字符串解析为抽象语法树（AST），并遍历其中的项
fn scan_file(content: &str, result: &mut Vec<RouteFunction>) {
    let file = parse_file(content).expect("Failed to parse file content");

    for item in file.items {
        process_item(&item, result);
    }
}

/// 处理 AST 中的每一项（函数或模块），尝试提取路由信息
fn process_item(item: &syn::Item, result: &mut Vec<RouteFunction>) {
    match item {
        syn::Item::Fn(fn_item) => {
            if let Some(route_fn) = extract_route_info(fn_item) {
                println!("✅ Found route function: {}", route_fn.name);
                result.push(route_fn);
            }
        }
        syn::Item::Mod(module) => {
            if let Some((_, ref items)) = module.content {
                for inner_item in items {
                    if let syn::Item::Fn(fn_item) = inner_item {
                        if let Some(route_fn) = extract_route_info(fn_item) {
                            println!("✅ Found route function: {}", route_fn.name);
                            result.push(route_fn);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// 表示一个发现的路由函数的信息
struct RouteFunction {
    name: String,       // 函数名称
    method: String,     // HTTP 方法（如 get、post）
    route_path: String, // 路由路径（如 /api/test）
}

/// 支持的 HTTP 方法列表
const METHOD_MAP: &[(&str, &str)] = &[
    ("get", "get"),
    ("post", "post"),
    ("put", "put"),
    ("delete", "delete"),
    ("head", "head"),
    ("connect", "connect"),
    ("options", "options"),
    ("trace", "trace"),
    ("patch", "patch"),
];

/// 提取函数上的方法属性（如 #[get(...)]）和文档注释
fn extract_route_info(fn_item: &ItemFn) -> Option<RouteFunction> {
    let mut method = None;
    let mut path = None;

    for attr in &fn_item.attrs {
        if is_route_attribute(attr) {
            if let Some((m, p)) = parse_route_attribute(attr) {
                method = Some(m);
                path = Some(p);
            }
        }
    }

    let name = fn_item.sig.ident.to_string();
    let method = method?;
    let route_path = path?;

    Some(RouteFunction {
        name,
        method,
        route_path,
    })
}

/// 判断属性是否是 actix-web 支持的 HTTP 方法属性（如 #[get(...)]）
fn is_route_attribute(attr: &syn::Attribute) -> bool {
    METHOD_MAP.iter().any(|&(k, _)| attr.path().is_ident(k))
}

/// 解析路由属性宏的方法和路径
fn parse_route_attribute(attr: &syn::Attribute) -> Option<(String, String)> {
    let key = get_attr_key(attr)?;
    let attr_path = attr.parse_args::<LitStr>().ok()?;
    let value = attr_path.value();
    METHOD_MAP
        .iter()
        .find(|&&(k, _)| k == key)
        .map(|&(_, v)| (v.to_string(), value))
}

/// 提取属性宏的标识符名称
fn get_attr_key(attr: &syn::Attribute) -> Option<String> {
    let segments: Vec<_> = attr.path().segments.iter().collect();
    if segments.len() == 1 {
        let ident = segments[0].ident.to_string();
        return Some(ident.to_lowercase());
    }
    None
}
